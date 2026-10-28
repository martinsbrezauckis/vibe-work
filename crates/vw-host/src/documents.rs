//! Files entering the app: the native picker's and an OS drop's paths go from the Tauri
//! side straight to the core here; the webview only ever sees document ids and facts.
//!
//! Depends on: vw-core-api (`Core::add_documents`, document views), `host`. Called by: `app`
//! (`pick_documents` and `pick_library_files` commands, the window's drag-and-drop handler), the private test host's
//! stand-in.
//! Invariant: the paths given here come from the OS (dialog or drop), never from a client request;
//! there is no command or route that reaches this with a caller-chosen path.

use std::path::PathBuf;

use vw_core_api::{CoreError, DocumentRefusal, DocumentSource, DocumentView};

use crate::host::Host;

/// What adding files did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentsPicked {
    /// Added documents (ids and facts).
    pub added: Vec<DocumentView>,
    /// Files not added, with the reason.
    pub refused: Vec<DocumentRefusal>,
}

impl Host {
    /// Takes waiting OS hand-overs through the native attachment reader. The window places the results.
    ///
    /// # Errors
    /// `serverUnavailable` without a core; waiting hand-overs remain in the inbox then.
    pub async fn take_hand_overs(
        &self,
        inbox: &crate::hand_over::HandOverInbox,
    ) -> Result<Vec<crate::hand_over::HandOverView>, CoreError> {
        let core = self.core_or_unavailable()?;
        let mut views = Vec::new();
        for hand_over in inbox.take() {
            views.push(crate::hand_over::HandOverView::read(hand_over, core).await);
        }
        Ok(views)
    }

    /// Uploads files the native picker gave into a Mistral library (paths from the OS only).
    ///
    /// # Errors
    /// `serverUnavailable` without a core; the core's errors.
    pub async fn upload_library_paths(
        &self,
        library_id: &str,
        paths: Vec<PathBuf>,
    ) -> Result<vw_core_api::CommandResult, CoreError> {
        let core = self.core_or_unavailable()?;
        core.upload_library_paths(library_id, paths).await
    }

    /// Adds files the native picker or an OS drop gave and returns what happened (the core also
    /// publishes `documentsAdded`, which is how the UI learns about a drop).
    ///
    /// # Errors
    /// `serverUnavailable` without a core.
    pub async fn add_documents(
        &self,
        paths: Vec<PathBuf>,
        via: DocumentSource,
    ) -> Result<DocumentsPicked, CoreError> {
        let core = self.core_or_unavailable()?;
        let (added, refused) = core.add_documents(paths, via).await;
        Ok(DocumentsPicked { added, refused })
    }
}
