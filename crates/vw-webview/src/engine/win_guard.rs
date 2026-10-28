//! Windows parts of a preview's network check: previews are refused, with a plain message naming why,
//! when every `WebView2` window of the app is put into ONE user data folder, and the development-build fault switch
//! real-webview fixture uses.
//!
//! Why refuse: `WebView2` takes the guard's proxy from the browser arguments of a user data folder's single browser
//! process, and every environment on that folder must use identical arguments (Microsoft Learn,
//! `ICoreWebView2Environment::CreateCoreWebView2Controller`: `HRESULT_FROM_WIN32(ERROR_INVALID_STATE)`). The
//! `WEBVIEW2_USER_DATA_FOLDER` variable and the `UserDataFolder` policy replace the folder of EVERY environment of the
//! app with one folder (Microsoft Learn, `CreateCoreWebView2EnvironmentWithOptions`), the main window's included, so
//! no preview can have a folder (and so a listener) of its own there, and no `WebView2` API sets a proxy per profile
//! or per webview. Without the proxy a preview's `WebSockets` reach this computer and the local network unchecked (the
//! request barrier does not see them: `WebView2Feedback` #4303), so such a preview is not opened (never
//! silently unguarded).
//!
//! Depends on: vw-preview (`EngineError`, `MountSpec`), vw-netguard, vw-platform (the folder setting check),
//! `folder_guard`. Called by: the engine's `proxy_for` and `mount_inner`.

use vw_preview::{EngineError, MountSpec};

use super::{TauriEngine, folder_guard, mac_guard::not_checked};

/// What puts every `WebView2` window of the app into one user data folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) enum OneFolder {
    /// The `WEBVIEW2_USER_DATA_FOLDER` environment variable.
    Variable,
    /// The `UserDataFolder` policy (registry, the machine's or the user's) for Vibe Work.
    Policy,
    /// An `AdditionalBrowserArguments` policy for Vibe Work (or every app) with a proxy switch: `WebView2` appends it
    /// after the guard's arguments, so it replaces the guard's proxy.
    ProxyPolicy,
    /// `WebView2` refused the guard's browser arguments: another environment with other arguments already uses the
    /// folder (a forcing setting the check above did not see).
    Refused,
}

/// The plain refusal of a preview that cannot have its network check because of `why`.
pub(super) fn previews_off(why: OneFolder) -> EngineError {
    EngineError::Platform(
        match why {
            OneFolder::Variable => {
                "Previews are off: the WEBVIEW2_USER_DATA_FOLDER setting puts every Vibe Work window into one \
                 WebView2 data folder, so a page's network check cannot be set up. Start Vibe Work without that \
                 setting to use previews."
            }
            OneFolder::Policy => {
                "Previews are off on this computer: a company policy (WebView2 UserDataFolder) puts every Vibe Work \
                 window into one WebView2 data folder, so a page's network check cannot be set up. Ask whoever \
                 manages this computer to remove that policy for Vibe Work."
            }
            OneFolder::ProxyPolicy => {
                "Previews are off on this computer: a company policy (WebView2 AdditionalBrowserArguments) changes \
                 WebView2's proxy settings for Vibe Work, so a page's network check cannot be set up. Ask whoever \
                 manages this computer to remove the proxy switches from that policy for Vibe Work."
            }
            OneFolder::Refused => {
                "Previews are off: another WebView2 window with other settings uses the same data folder as this \
                 preview, so its network check cannot be set up. Restart Vibe Work; if it keeps happening, a \
                 WebView2 setting on this computer puts every window into one folder."
            }
        }
        .to_owned(),
    )
}

/// True when a webview creation failed because another environment with other browser arguments uses the folder:
/// `HRESULT_FROM_WIN32(ERROR_INVALID_STATE)` = `0x8007139F` in the error text (wry passes the `WebView2` error on).
pub(super) fn other_arguments(error: &EngineError) -> bool {
    error.to_string().to_ascii_uppercase().contains("8007139F")
}

/// `VW_DEV_WIN_GUARD_FAULT=folder-check` (Windows development builds only; the real-webview fixture): the setting check
/// reports no forced folder, so a mount meets `WebView2`'s own refusal of different arguments on one folder.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn win_fault(kind: &str) -> bool {
    cfg!(all(windows, debug_assertions))
        && std::env::var("VW_DEV_WIN_GUARD_FAULT").is_ok_and(|v| v == kind)
}

impl TauriEngine {
    /// Windows: the proxy URL of a new mount (the listener of its user data folder), a refusal on the previews'
    /// instance when no preview can have it, or `None` on another instance (`proxy_for`).
    pub(super) fn windows_proxy(
        &self,
        guard: &vw_netguard::NetGuard,
        spec: &MountSpec,
    ) -> Result<Option<String>, EngineError> {
        let id = &spec.id;
        // Windows: one data folder for every window leaves no preview a proxy of its own: refused (fail closed).
        if let Some(why) = self.one_folder() {
            tracing::warn!(
                ?why,
                "every WebView2 window shares one user data folder: no guard proxy for previews"
            );
            return if self.needs_guard {
                Err(previews_off(why))
            } else {
                Ok(None)
            };
        }
        match guard.proxy_url_for_store(&folder_guard::store_key(&spec.storage.dir), &id.label) {
            Ok(url) => Ok(Some(url)),
            Err(error) => {
                tracing::warn!(%error, "the browser's network guard listener did not start");
                if self.needs_guard {
                    Err(not_checked("its network check did not start"))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Windows: what puts every `WebView2` window of the app into one user data folder (`None`: nothing; each preview
    /// folder then gets its own browser process and the guard's arguments).
    #[cfg(windows)]
    pub(super) fn one_folder(&self) -> Option<OneFolder> {
        if self
            .proxy_args_refused
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Some(OneFolder::Refused);
        }
        if win_fault("folder-check") {
            return None;
        }
        match vw_platform::webview::user_data_folder_override() {
            Some(vw_platform::webview::FolderOverride::Variable) => Some(OneFolder::Variable),
            Some(vw_platform::webview::FolderOverride::Policy) => Some(OneFolder::Policy),
            None if vw_platform::webview::policy_arguments_change_proxy() => {
                Some(OneFolder::ProxyPolicy)
            }
            None => None,
        }
    }

    #[cfg(not(windows))]
    #[allow(clippy::unused_self)]
    pub(super) fn one_folder(&self) -> Option<OneFolder> {
        None
    }
}
