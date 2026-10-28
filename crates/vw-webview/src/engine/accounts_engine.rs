//! Lazy native account-store anchor for the shared browser profile.
//!
//! Depends on: Tauri child webviews, native layout and `WebView2` profile facts.
//! Called by: `TauriEngine` account operations. Invariant: the anchor loads only `about:blank`.

#[cfg(any(windows, target_os = "linux"))]
use std::time::Duration;

use tauri::webview::{NewWindowResponse, PermissionResponse};
use tauri::{LogicalPosition, LogicalSize, Manager, Webview, WebviewBuilder, WebviewUrl, Wry};
use url::Url;
use vw_preview::{EngineError, Storage};

use super::TauriEngine;
#[cfg(windows)]
use super::builder::with_proxy;
#[cfg(any(windows, target_os = "linux"))]
use crate::platform;

impl TauriEngine {
    /// Creates a hidden, blank native webview in the shared profile so account checks work before a page opens.
    pub(super) async fn account_anchor(
        &self,
        storage: &Storage,
    ) -> Result<Webview<Wry>, EngineError> {
        if !storage.is_shared_web() {
            return Err(EngineError::Platform(
                "account operations require browser storage".to_owned(),
            ));
        }
        let mut anchor = self.account_anchor.lock().await;
        if let Some(webview) = anchor.as_ref() {
            return Ok(webview.clone());
        }
        let window = self
            .app
            .get_window(&self.window_label)
            .ok_or_else(|| EngineError::Platform("the main window is not open".to_owned()))?;
        let blank = Url::parse("about:blank").map_err(|e| EngineError::Platform(e.to_string()))?;
        let builder = WebviewBuilder::new("vw-preview-account-anchor", WebviewUrl::External(blank))
            .focused(false)
            .general_autofill_enabled(false)
            .devtools(false)
            .on_navigation(|url| url.as_str() == "about:blank")
            .on_new_window(|_, _| NewWindowResponse::Deny)
            .on_permission_request(|_, _| PermissionResponse::Deny);
        let builder = crate::storage::with_storage(builder, storage);
        #[cfg(windows)]
        // No forced single folder: the anchor takes the web folder's arguments, as the web previews on it will.
        let builder = if self.one_folder().is_none() {
            match self
                .guard
                .as_ref()
                // The shared web folder's listener, without making the anchor a member (it never loads a page).
                .and_then(|guard| {
                    guard
                        .store_proxy_url(&super::folder_guard::store_key(&storage.dir))
                        .ok()
                }) {
                Some(proxy) => with_proxy(builder, &proxy)?,
                None => builder,
            }
        } else {
            builder
        };
        let webview = tokio::task::spawn_blocking(move || {
            window.add_child(
                builder,
                LogicalPosition::new(-10010.0, -10010.0),
                LogicalSize::new(1.0, 1.0),
            )
        })
        .await
        .map_err(|e| EngineError::Platform(e.to_string()))?
        .map_err(|e| EngineError::Platform(format!("account browser storage: {e}")))?;
        let setup = async {
            #[cfg(target_os = "linux")]
            platform::with_native(
                &webview,
                "parking the account browser",
                Duration::from_secs(5),
                |native, reply| {
                    let _ = reply.send(crate::linux_layout::park(&native.inner(), 1, 1));
                },
            )
            .await?;
            #[cfg(not(target_os = "linux"))]
            webview
                .hide()
                .map_err(|e| EngineError::Platform(e.to_string()))?;
            #[cfg(windows)]
            {
                let expected = storage.key.hex();
                let facts = platform::with_native(
                    &webview,
                    "checking account profile",
                    Duration::from_secs(5),
                    move |native, reply| {
                        let result = vw_platform::webview::storage_facts(
                            &native.controller(),
                            &native.environment(),
                        );
                        let _ = reply.send(result);
                    },
                )
                .await?;
                if !facts.profile_name.eq_ignore_ascii_case(&expected) {
                    return Err(EngineError::Platform(
                        "the account browser did not get its own storage profile".to_owned(),
                    ));
                }
            }
            Ok::<(), EngineError>(())
        }
        .await;
        if let Err(error) = setup {
            let _ = webview.close();
            return Err(error);
        }
        *anchor = Some(webview.clone());
        Ok(webview)
    }
}
