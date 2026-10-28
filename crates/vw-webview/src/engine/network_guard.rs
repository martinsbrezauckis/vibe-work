//! Listener selection and bounded canary proof before a preview's first page.
//! Depends on the Tauri engine and vw-netguard; called by `mount_inner`. Required mounts always fail closed.

use super::mac_guard::{MAC_PROXY, mac_fault, not_checked, store_key};
use super::{TauriEngine, folder_guard};
use std::sync::Arc;
use std::time::Duration;
use tauri::{Webview, Wry};
use url::Url;
use vw_preview::{EngineError, GuardCheck, MountId, MountSpec, PreviewHooks};

/// End macOS canary navigation before reporting it (`WebKit` asks about the URL again while finishing).
const CANARY_SETTLE: Duration = Duration::from_millis(300);
/// One budget for both canaries: cold engines may take over four seconds to send the first one.
const CANARY_WAIT: Duration = Duration::from_secs(6);

impl TauriEngine {
    /// Whether this preview requires a proxy listener and canary proof before its first page.
    pub(super) fn proxy_required(&self) -> bool {
        proxy_required_for(self.needs_guard, cfg!(target_os = "macos"), MAC_PROXY)
    }

    /// The proxy URL a new mount gets (`None`: this engine does not require an egress layer).
    /// - Linux: the listener of the mount's data folder (`folder_guard`): `WebKitGTK` takes the proxy per web context,
    ///   which Tauri keeps one per folder, so the last mount's setting would carry every page of the folder; the
    ///   folder's listener grants only what every mount on it may reach. `WebKitGTK` sends every request, loopback
    ///   included, through the proxy.
    /// - Windows: the listener of the mount's user data folder (`folder_guard`), the same for every mount on that
    ///   folder, because every environment that shares a `WebView2` browser process must have identical browser
    ///   arguments; it grants only what every mount on the folder may reach, and the per-webview request barrier
    ///   keeps each mount to its own rights for HTTP. Where a setting puts every environment into one user data
    ///   folder, or after `WebView2` refused the arguments, or when the listener cannot start, the previews'
    ///   instance (`require_network_guard`) refuses the mount with a plain message (`win_guard`); another instance
    ///   gets no proxy.
    /// - macOS with `macos-proxy` ([`MAC_PROXY`]): the listener of the mount's website data store (the store carries
    ///   the setting; the shared web store serves several mounts), the same port for the store's life; the content
    ///   rules keep each mount to its own rights for literals. A listener that cannot start, or no guard at all on the
    ///   previews' instance, refuses the mount (fail closed). Without the feature (0.1.2), macOS previews use content
    ///   rules only, including required previews; a failed rule install refuses the mount.
    pub(super) fn proxy_for(&self, spec: &MountSpec) -> Result<Option<String>, EngineError> {
        let id = &spec.id;
        let Some(guard) = &self.guard else {
            if self.proxy_required() {
                return Err(not_checked("the network check is not running"));
            }
            return Ok(None);
        };
        if cfg!(target_os = "linux") {
            return guard
                .proxy_url_for_store(&folder_guard::store_key(&spec.storage.dir), &id.label)
                .map(Some)
                .map_err(|error| {
                    tracing::warn!(%error, "the browser's network guard listener did not start");
                    not_checked("its network check did not start")
                });
        }
        if MAC_PROXY {
            // Fail closed: no listener, no page.
            if mac_fault("listener") {
                return Err(not_checked("its network check did not start"));
            }
            return guard
                .proxy_url_for_store(&store_key(spec), &id.label)
                .map(Some)
                .map_err(|error| {
                    tracing::warn!(%error, "the browser's network guard listener did not start");
                    not_checked("its network check did not start")
                });
        }
        if !cfg!(windows) {
            if self.proxy_required() {
                return Err(not_checked("this build has no guarded preview proxy"));
            }
            return Ok(None);
        }
        self.windows_proxy(guard, spec)
    }

    /// Proves that the fresh mount's traffic passes the guard (Linux, Windows): a request for a
    /// public-looking name (`*.vw-guard.invalid`, only the proxy can answer it) and a navigation to the listener's
    /// own port on loopback, which reaches the guard in proxy form only when the engine does not bypass the proxy
    /// for loopback. macOS: the name canary only, and only with WebTransport off (`bypass_closed`): `WebKit` sends
    /// address literals around the proxy by design (measured on macOS 27.0.1: `127.0.0.1`, `[::1]`, `localhost` and
    /// this computer's own LAN address went direct, names and other literals through the proxy), so a loopback
    /// canary can never arrive there; the content rules cover literals. The result is reported
    /// (`network_guard(Done(..))`) and returned; an unproven required proxy refuses the mount (the caller).
    /// Default macOS builds use content rules only and do not require this proof.
    pub(super) async fn check_guard(
        &self,
        webview: &Webview<Wry>,
        id: &MountId,
        proxy: Option<&str>,
        bypass_closed: bool,
        hooks: &Arc<dyn PreviewHooks>,
    ) -> bool {
        let (Some(guard), Some(proxy)) = (&self.guard, proxy) else {
            return false;
        };
        // Each canary is announced to the hooks first (the only canary URL the mount may load), then loaded.
        let load = |text: &str| match Url::parse(text) {
            Ok(url) => {
                hooks.network_guard(&id.label, GuardCheck::Expect(url.clone()));
                webview.navigate(url).is_ok()
            }
            Err(_) => false,
        };
        let deadline = tokio::time::Instant::now() + CANARY_WAIT;
        let token = vw_netguard::NetGuard::new_token();
        let web = load(&vw_netguard::NetGuard::canary_url(&token))
            && guard
                .wait_seen(
                    &token,
                    deadline.saturating_duration_since(tokio::time::Instant::now()),
                )
                .await
            && !mac_fault("canary");
        // macOS: the guard sees the canary at its CONNECT, before WebKit has finished that navigation; WebKit then
        // asks about the canary URL once more. Leave it for about:blank while the canary is still the expected URL,
        // so that late question never meets an ended check (real app: a "Not opened ... vw-guard.invalid" banner).
        if cfg!(target_os = "macos")
            && web
            && let Ok(blank) = Url::parse("about:blank")
        {
            let _ = webview.navigate(blank);
            tokio::time::sleep(CANARY_SETTLE).await;
        }
        let loopback = if cfg!(target_os = "macos") {
            // Literals bypass the proxy on macOS (see above): the proof is the name canary and WebTransport off.
            web && bypass_closed
        } else {
            let token = vw_netguard::NetGuard::new_token();
            let port = Url::parse(proxy).ok().and_then(|u| u.port()).unwrap_or(0);
            web && load(&vw_netguard::NetGuard::loopback_canary_url(proxy, &token))
                && guard
                    .wait_seen_loopback(
                        &token,
                        port,
                        deadline.saturating_duration_since(tokio::time::Instant::now()),
                    )
                    .await
        };
        if !loopback {
            tracing::warn!(
                web,
                bypass_closed,
                "the browser's network guard is not proven for this webview"
            );
        }
        hooks.network_guard(&id.label, GuardCheck::Done(web && loopback));
        web && loopback
    }
}

/// On macOS, 0.1.2 keeps content-rules-only previews; the proxy is required with
/// `macos-proxy` (0.1.3, minimum macOS 14). Explicit platform inputs let every build test that policy.
fn proxy_required_for(needs_guard: bool, macos: bool, mac_proxy: bool) -> bool {
    needs_guard && (!macos || mac_proxy)
}

#[cfg(test)]
mod tests {
    use super::proxy_required_for;

    #[test]
    fn linux_and_windows_require_proxy_proof() {
        // Neither platform gets the macOS exception, even with the macos-proxy feature enabled.
        for mac_proxy in [false, true] {
            assert!(proxy_required_for(true, false, mac_proxy));
            assert!(!proxy_required_for(false, false, mac_proxy));
        }
    }

    #[test]
    fn macos_default_uses_content_rules_only() {
        assert!(!proxy_required_for(true, true, false));
        assert!(!proxy_required_for(false, true, false));
    }

    #[test]
    fn macos_proxy_requires_proxy_proof() {
        assert!(proxy_required_for(true, true, true));
        assert!(!proxy_required_for(false, true, true));
    }
}
