//! The work-tool calls of the key service: document Libraries and image generation. Like OCR and
//! speech they go straight to Mistral (`test_api_base`, normally `https://api.mistral.ai`) with the key in
//! use, not through the model proxy.
//!
//! Depends on: vw-mistral (`work::libraries`, `work::images`), `service` (`key_for_call`, `api`). Called by:
//! vw-core-api (`libraries`, `images`, the agent's `library_search` and `image_generate` tools).
//! Invariants: one-way (no function returns the key); without a key the unlock or setup prompt is raised and the
//! call fails at once (no waiting, no retry behind the user's back).

use vw_keyproxy::KeyNeed;
use vw_mistral::work::WorkError;
use vw_mistral::work::images::{ImageJob, Made};
use vw_mistral::work::libraries::{self, Answer, Library, LibraryDocument, LibraryList};

use crate::service::KeyService;

/// Why a work call did not run or failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkFailure {
    /// No key is available: the vault is locked or no key is set up (the prompt was raised).
    #[error("{}", match .0 {
        KeyNeed::Unlock => "Unlock your vault first (Vibe Work needs your Mistral key for this), then try again.",
        KeyNeed::Setup => "Set up your Mistral API key first (Settings), then try again.",
    })]
    KeyNeeded(KeyNeed),
    /// Mistral answered with an error, or the answer was refused.
    #[error(transparent)]
    Work(WorkError),
}

impl From<WorkError> for WorkFailure {
    fn from(error: WorkError) -> Self {
        Self::Work(error)
    }
}

impl KeyService {
    /// The libraries the key can see.
    ///
    /// # Errors
    /// [`WorkFailure`].
    pub async fn libraries(&self) -> Result<LibraryList, WorkFailure> {
        let key = self.key_for_call().map_err(WorkFailure::KeyNeeded)?;
        let (client, base) = self.api();
        Ok(libraries::list(client, base, key.as_str()).await?)
    }

    /// Creates a library.
    ///
    /// # Errors
    /// [`WorkFailure`].
    pub async fn library_create(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<Library, WorkFailure> {
        let key = self.key_for_call().map_err(WorkFailure::KeyNeeded)?;
        let (client, base) = self.api();
        Ok(libraries::create(client, base, key.as_str(), name, description).await?)
    }

    /// Deletes a library and its documents.
    ///
    /// # Errors
    /// [`WorkFailure`].
    pub async fn library_delete(&self, id: &str) -> Result<(), WorkFailure> {
        let key = self.key_for_call().map_err(WorkFailure::KeyNeeded)?;
        let (client, base) = self.api();
        Ok(libraries::delete(client, base, key.as_str(), id).await?)
    }

    /// The documents of a library.
    ///
    /// # Errors
    /// [`WorkFailure`].
    pub async fn library_documents(&self, id: &str) -> Result<Vec<LibraryDocument>, WorkFailure> {
        let key = self.key_for_call().map_err(WorkFailure::KeyNeeded)?;
        let (client, base) = self.api();
        Ok(libraries::documents(client, base, key.as_str(), id).await?)
    }

    /// Uploads one file (`name`, MIME type, bytes) into a library.
    ///
    /// # Errors
    /// [`WorkFailure`].
    pub async fn library_upload(
        &self,
        library: &str,
        file: (&str, &str, &[u8]),
    ) -> Result<LibraryDocument, WorkFailure> {
        let key = self.key_for_call().map_err(WorkFailure::KeyNeeded)?;
        let (client, base) = self.api();
        Ok(libraries::upload(client, base, key.as_str(), library, file).await?)
    }

    /// Deletes one document of a library.
    ///
    /// # Errors
    /// [`WorkFailure`].
    pub async fn library_document_delete(
        &self,
        library: &str,
        document: &str,
    ) -> Result<(), WorkFailure> {
        let key = self.key_for_call().map_err(WorkFailure::KeyNeeded)?;
        let (client, base) = self.api();
        Ok(libraries::delete_document(client, base, key.as_str(), library, document).await?)
    }

    /// The text Mistral extracted from a document, and whether it was cut.
    ///
    /// # Errors
    /// [`WorkFailure`].
    pub async fn library_document_text(
        &self,
        library: &str,
        document: &str,
    ) -> Result<(String, bool), WorkFailure> {
        let key = self.key_for_call().map_err(WorkFailure::KeyNeeded)?;
        let (client, base) = self.api();
        Ok(libraries::document_text(client, base, key.as_str(), library, document).await?)
    }

    /// Asks `question` across `libraries` with `model`.
    ///
    /// # Errors
    /// [`WorkFailure`].
    pub async fn library_search(
        &self,
        model: &str,
        question: &str,
        libraries: &[String],
    ) -> Result<Answer, WorkFailure> {
        let key = self.key_for_call().map_err(WorkFailure::KeyNeeded)?;
        let (client, base) = self.api();
        Ok(libraries::search(client, base, key.as_str(), model, question, libraries).await?)
    }

    /// Makes one image (new or edited). The download uses the same no-redirect client, without the key.
    ///
    /// # Errors
    /// [`WorkFailure`].
    pub async fn image_make(&self, job: &ImageJob<'_>) -> Result<Made, WorkFailure> {
        let key = self.key_for_call().map_err(WorkFailure::KeyNeeded)?;
        let (client, base) = self.api();
        Ok(vw_mistral::work::images::make_with_cleanup_notice(
            client,
            base,
            key.as_str(),
            job,
            self.cleanup_notice_handler(),
        )
        .await?)
    }
}
