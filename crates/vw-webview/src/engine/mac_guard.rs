//! macOS parts of a preview's network check: the content rules before the first page and, with the feature
//! `macos-proxy` ([`MAC_PROXY`]), WebTransport off, the plain refusal when the check is not in place (fail closed)
//! and the development-build fault switch the Mac review runs use. Without the feature (0.1.2: minimum macOS 11)
//! all previews use content rules only, including required previews; failed rule installs refuse the mount.
//! macOS preview network guard: 0.1.3 with minimum 14.
//! Depends on: `rules_macos`, `platform` (macOS). Called by: the engine's mount (`proxy_for`, `check_guard`,
//! `mount_inner`).

#[cfg(target_os = "macos")]
use std::sync::Arc;

#[cfg(target_os = "macos")]
use tauri::{Webview, Wry};
use vw_preview::EngineError;
#[cfg(target_os = "macos")]
use vw_preview::{GuardCheck, MountSpec, PreviewHooks};

use super::TauriEngine;
#[cfg(target_os = "macos")]
use crate::platform;

/// macOS previews go through the guard's proxy (the store's listener, `WKWebsiteDataStore.proxyConfigurations`,
/// Network.framework's macOS 14 API): only in builds with the feature `macos-proxy`.
pub(super) const MAC_PROXY: bool = cfg!(all(target_os = "macos", feature = "macos-proxy"));

/// `VW_DEV_MAC_GUARD_FAULT=<listener|webtransport|canary|rules>` (macOS development builds only; review runs):
/// pretends that the store's guard listener did not start, that WebTransport could not be turned off, or that the
/// canary was not seen, so the real app shows what a preview does in each failure.
pub(crate) fn mac_fault(kind: &str) -> bool {
    cfg!(all(target_os = "macos", debug_assertions))
        && std::env::var("VW_DEV_MAC_GUARD_FAULT").is_ok_and(|v| v == kind)
}

/// The plain refusal of a preview whose required network check is not in place (every platform).
pub(super) fn not_checked(why: &str) -> EngineError {
    EngineError::Platform(format!(
        "Vibe Work could not set up this page's network check ({why}), so the page was not opened. Try again; \
         if it keeps happening, restart Vibe Work."
    ))
}

/// macOS: whether `spec` gets a data store of its own (a non-persistent one, per webview): the local-network page the
/// user opened. Its store's guard listener then has this mount as its only member, so its Lan host is never shared
/// with the session's own-server pages (which share the session's store) and theirs never with it (the guard grants
/// a store only what every member may reach). Cost: such pages keep no cookies across
/// remounts (macOS 11-13 treated local pages the same way). Only behind the proxy ([`MAC_PROXY`]); without it the
/// page keeps its session's store, as in 0.1.1.
pub(super) fn own_store(spec: &vw_preview::MountSpec) -> bool {
    MAC_PROXY && matches!(spec.rules, Some(vw_netguard::rules::RulePage::Lan { .. }))
}

/// The guard's store key of `spec`'s data store: the storage key, or the mount's own (`own_store`).
pub(super) fn store_key(spec: &vw_preview::MountSpec) -> String {
    if own_store(spec) {
        own_store_key(&spec.id.label)
    } else {
        spec.storage.key.hex()
    }
}

/// The guard's store key of a mount with a data store of its own.
pub(super) fn own_store_key(label: &str) -> String {
    format!("mount:{label}")
}

impl TauriEngine {
    /// macOS, before the first page: the mount's content rules (the mount waits for the compile) and, for a mount
    /// behind the guard's proxy (`proxied`), WebTransport off (it ignores the store's proxy). WebTransport that
    /// cannot be turned off refuses the mount (fail closed).
    /// A missing content-rule layer refuses the mount, including in development fault runs.
    #[cfg(target_os = "macos")]
    pub(super) async fn macos_layers(
        &self,
        webview: &Webview<Wry>,
        spec: &MountSpec,
        hooks: &Arc<dyn PreviewHooks>,
        proxied: bool,
    ) -> Result<(), EngineError> {
        if let Some(page) = spec.rules.as_ref() {
            content_rules_result(
                crate::rules_macos::apply(webview, &spec.id.label, page, &self.rule_generations)
                    .await,
            )?;
            hooks.network_guard(
                &spec.id.label,
                GuardCheck::Layer(vw_preview::GuardLayer::ContentRules),
            );
        }
        if !proxied {
            return Ok(());
        }
        if mac_fault("webtransport") {
            return Err(not_checked("WebTransport could not be turned off"));
        }
        platform::web_transport_off(webview).await.map_err(|error| {
            tracing::warn!(%error, "WebTransport could not be turned off");
            not_checked("WebTransport could not be turned off")
        })
    }
}

/// Converts the native rule-install result into the mount decision on every build target.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn content_rules_result(result: Result<(), EngineError>) -> Result<(), EngineError> {
    result.map_err(|error| {
        tracing::warn!(%error, "preview content rules not applied; refusing the mount");
        not_checked("its content rules could not be installed")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_rules_failure_refuses_the_mount() {
        for why in [
            "compile failed",
            "main thread unavailable",
            "compile timed out",
        ] {
            let result = content_rules_result(Err(EngineError::Platform(why.to_owned())));
            assert!(result.is_err(), "mount opened after {why}");
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("so the page was not opened")
            );
        }
        assert!(content_rules_result(Ok(())).is_ok());
    }
}
