//! Shared browser storage and provider account operations for `PreviewService`.
//!
//! Depends on: the engine's native account seam. Called by: core account commands.
//! Invariant: only the shared public-browser store is inspected or changed.

use crate::accounts::{Provider, ProviderAccount};
use crate::engine::{EngineError, EngineFuture, Storage};

use super::{PreviewError, PreviewService, bounded};

impl PreviewService {
    /// Bounds an engine operation by the service's operation limit.
    pub(crate) async fn bounded<T>(
        &self,
        what: &str,
        future: EngineFuture<'_, T>,
    ) -> Result<T, EngineError> {
        bounded(self.config.engine_timeout, what, future).await
    }

    /// Dedicated persistent website data shared by public browser pages in every session.
    #[must_use]
    pub fn shared_web_storage(&self) -> Storage {
        let key = crate::ids::StorageKey::shared_web();
        Storage {
            key,
            dir: key.data_dir(&self.config.storage_root),
        }
    }

    /// Reports provider session-cookie presence from the native browser store.
    ///
    /// # Errors
    /// The native engine is unavailable or its cookie query fails or times out.
    pub async fn accounts(&self) -> Result<Vec<ProviderAccount>, PreviewError> {
        let storage = self.shared_web_storage();
        self.bounded("accounts", self.engine.accounts(&storage))
            .await
            .map_err(Into::into)
    }

    /// Deletes provider-domain cookies from the native browser store.
    ///
    /// # Errors
    /// The native engine is unavailable or deletion fails or times out.
    pub async fn sign_out(&self, provider: Provider) -> Result<(), PreviewError> {
        let storage = self.shared_web_storage();
        self.bounded("sign_out", self.engine.sign_out(&storage, provider))
            .await
            .map_err(Into::into)
    }
}
