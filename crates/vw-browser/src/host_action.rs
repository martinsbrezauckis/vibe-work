//! Routes host browser controls separately from page input and its DOM receipt.
//! Called inside the common action loop, retaining Stop, ownership and action logging.
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
