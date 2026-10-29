//! The app's theme in the handbook viewer (Handbook Format 1.2 §6.4): the viewer sets `<html
//! data-theme="light|dark">` on every page it loads, from a document-start script of its engine (exempt from the
//! page's CSP, runs no page code), so pages whose stylesheet keys its tokens on `data-theme` match the app.
//!
//! Depends on: `service` (viewer mount, engine calls), `handbook::format`. Called by: vw-core-api (`theme` request),
//! `service` (mount).

use crate::config::PagesError;
use crate::service::{PagesService, bounded, mount_id};

impl PagesService {
    /// The app theme the viewer passes to pages (`light` or `dark`).
    #[must_use]
    pub fn theme(&self) -> String {
        self.theme
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Sets the app theme the viewer passes to pages (anything but `light` is `dark`, the app's default). A change
    /// remounts the viewer, so the page loads again with the new theme from its first paint (a prototype's own state
    /// is lost; theme changes are rare).
    ///
    /// # Errors
    /// Engine failures while showing the page again.
    pub async fn set_theme(&self, theme: &str) -> Result<(), PagesError> {
        let theme = if theme == "light" { "light" } else { "dark" };
        {
            let mut current = self
                .theme
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *current == theme {
                return Ok(());
            }
            theme.clone_into(&mut current);
        }
        let current = self.shared.lock().mount.clone();
        if let Some(mount) = current {
            self.flush_inline(&mount).await?;
        }
        let mount = self.shared.lock().mount.take();
        if let Some(mount) = mount {
            let _ = bounded(
                self.config.engine_timeout,
                "close",
                self.engine.close(&mount_id(&mount)),
            )
            .await;
        }
        self.show_current(None).await?;
        Ok(())
    }
}

/// The document-start script that puts `theme` on `<html data-theme>` (the element may not exist yet when it runs:
/// then it waits for it).
#[must_use]
pub fn theme_script(theme: &str) -> String {
    let theme = if theme == "light" { "light" } else { "dark" };
    format!(
        "(function(){{var t='{theme}';var s=function(){{document.documentElement.setAttribute('{attr}',t)}};\
         if(document.documentElement){{s()}}else{{var o=new MutationObserver(function(){{if(document.documentElement)\
         {{s();o.disconnect()}}}});o.observe(document,{{childList:true}})}}}})();",
        attr = crate::handbook::format::THEME_ATTRIBUTE
    )
}
