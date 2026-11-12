//! Routes host browser controls separately from page input and its DOM receipt.
//! Called inside the common action loop, retaining Stop, ownership and action logging.
//! Depends on: browser navigation and download/dialog handlers.
//! Called by: the common browser action loop.
use crate::{ActError, Action, Actor, Browser, Navigate};
use serde_json::Value;
use std::sync::{Arc, atomic::AtomicBool};

impl Browser {
    pub(crate) async fn perform(
        &self,
        session: &str,
        action: &Action,
        nav: &dyn Navigate,
        consent: Option<&Value>,
        by: Actor,
        native_sent: Arc<AtomicBool>,
    ) -> Result<Value, ActError> {
        match action {
            Action::Recording(request) => self.recording(session, request, by).await,
            Action::TabAudio(muted) => {
                let bound = self
                    .bind_read(session, by)
                    .await
                    .map_err(crate::driver::refused)?;
                let mount = bound
                    .read_mount
                    .as_ref()
                    .ok_or_else(|| crate::driver::refused(crate::NOTHING_OPEN))?;
                let data = self
                    .service
                    .tab_audio(mount, *muted)
                    .await
                    .map_err(|e| crate::driver::failed(crate::driver::preview_text(&e)))?;
                bound.check_read_mount().map_err(crate::driver::refused)?;
                Ok(
                    serde_json::json!({"ok":true,"Outcome":"Native tab audio state read back.","action":action.name(),"untrustedPageContent":data}),
                )
            }
            Action::Media(request) => self.playback(session, request, by).await,
            Action::Network(request) => self.network(session, request, by).await,
            Action::PageScript(request) => self.page_script(session, request, by).await,
            Action::Viewport(request) => self.viewport(session, request, by).await,
            Action::Inspect(request) => self.inspect_element(session, request, by).await,
            Action::Diagnostics(request) => self.diagnostics(session, request, by).await,
            Action::ResolvePoint { x, y } => self.resolve_observed_point(session, *x, *y, by).await,
            Action::Dialog(request) => self.dialog_action(session, request, by).await,
            Action::Screenshot(area) => self.screenshot_area(session, area, by).await,
            Action::Downloads(request) => self.downloads(session, request).await,
            Action::Storage(request) => self.site_storage(session, request, by).await,
            Action::Cookies(request) => self.site_cookies(session, request, by).await,
            _ => {
                let before = self.service.chat_downloads(session);
                let mut receipt = self
                    .perform_page(session, action, nav, consent, by, native_sent)
                    .await?;
                let downloads: Vec<_> = self
                    .service
                    .chat_downloads(session)
                    .into_iter()
                    .filter(|d| !before.iter().any(|old| old["id"] == d["id"]))
                    .collect();
                if !downloads.is_empty() {
                    receipt["untrustedPageContent"]["downloads"] = serde_json::json!(downloads);
                }
                Ok(receipt)
            }
        }
    }
}
