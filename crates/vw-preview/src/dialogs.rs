//! Held native dialog access, pinned to one live browser mount; used by the browser tool and vault writer.
//! Answers never evaluate page JavaScript and credentials never enter a serialized receipt.
//! Depends on: preview mount ownership and native dialog engines.
//! Called by: browser tools and the vault authentication writer.
use crate::{EngineError, MountId, PreviewError, PreviewService};
use serde_json::Value;
use zeroize::Zeroizing;

/// Host-only native answer. Credentials come exclusively from the core's checked vault route.
#[derive(Default)]
pub struct DialogAnswer {
    /// Accept or dismiss the dialog.
    pub accept: bool,
    /// The supplied prompt text.
    pub text: Option<String>,
    /// Vault user name and password, excluded from serialization and debugging.
    pub credentials: Option<(Zeroizing<String>, Zeroizing<String>)>,
}

impl PreviewService {
    /// The native dialog currently holding this page, independent of its blocked JavaScript event loop.
    #[must_use]
    pub fn pending_dialog(&self, page: &str) -> Option<Value> {
        self.page_mount(page)
            .and_then(|id| self.engine.pending_dialog(&id))
    }

    /// Answers exactly an observed live mount and dialog.
    ///
    /// # Errors
    /// The page or dialog changed, or the native completion failed.
    pub async fn answer_dialog(
        &self,
        id: &MountId,
        dialog: &str,
        answer: DialogAnswer,
    ) -> Result<(), PreviewError> {
        if self.page_mount(&id.session_id).as_ref() != Some(id)
            || self
                .engine
                .pending_dialog(id)
                .is_none_or(|v| v["dialogId"] != dialog)
        {
            return Err(EngineError::NotMounted.into());
        }
        self.bounded(
            "answering page dialog",
            self.engine.answer_dialog(id, dialog.to_owned(), answer),
        )
        .await?;
        Ok(())
    }

    /// Dismiss held continuations on Stop so a page cannot keep the browser's input loop blocked.
    pub async fn dismiss_dialog(&self, page: &str) {
        if let Some(id) = self.page_mount(page)
            && let Some(dialog) = self.engine.pending_dialog(&id)
            && let Some(dialog) = dialog["dialogId"].as_str()
        {
            let _ = self
                .answer_dialog(&id, dialog, DialogAnswer::default())
                .await;
        }
    }
}
