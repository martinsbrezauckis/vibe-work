//! Typed skills requests. Depends on serde wire types and an injected transport;
//! app callers own sessions, runtime application, and refresh policy.

use std::future::Future;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    SkillsError, TransportError,
    wire::{
        SkillsCatalogParams, SkillsCatalogResponse, SkillsConvertLocalParams,
        SkillsConvertResponse, SkillsDetailParams, SkillsDetailResponse, SkillsImportParams,
        SkillsImportResponse, SkillsInstalledParams, SkillsInstalledResponse, SkillsListParams,
        SkillsListResponse, SkillsRemoveParams, SkillsRemoveResponse, SkillsSetAliasParams,
        SkillsSetAliasResponse, SkillsSetEnabledParams, SkillsSetEnabledResponse,
        SkillsSetLatestParams, SkillsSetLatestResponse, SkillsSetVersionParams,
        SkillsSetVersionResponse, SkillsUpdatesParams, SkillsUpdatesResponse, SkillsVersionsParams,
        SkillsVersionsResponse,
    },
};

/// The only channel boundary; successful values are unwrapped JSON-RPC results.
pub trait SkillsTransport: Send + Sync {
    /// Send one method and its params; return a result or a structured Vibe error.
    fn request(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, TransportError>> + Send;
}

/// A transport-free client; mutations return Vibe's complete runtime unchanged.
pub struct SkillsClient<T> {
    transport: T,
}

macro_rules! method {
    ($name:ident, $wire:literal, $params:ty, $result:ty) => {
        #[doc = concat!("Call `", $wire, "` with Vibe's exact params and result model.")]
        ///
        /// # Errors
        /// Returns Vibe's rejection, a channel failure, or a malformed response.
        pub async fn $name(&self, params: &$params) -> Result<$result, SkillsError> {
            self.call($wire, params).await
        }
    };
}

impl<T: SkillsTransport> SkillsClient<T> {
    /// Use the supplied channel adapter, without starting a process or reading disk.
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    method!(list, "skills/list", SkillsListParams, SkillsListResponse);
    method!(
        installed,
        "skills/installed",
        SkillsInstalledParams,
        SkillsInstalledResponse
    );
    method!(
        catalog,
        "skills/catalog",
        SkillsCatalogParams,
        SkillsCatalogResponse
    );
    method!(
        versions,
        "skills/versions",
        SkillsVersionsParams,
        SkillsVersionsResponse
    );
    method!(
        updates,
        "skills/updates",
        SkillsUpdatesParams,
        SkillsUpdatesResponse
    );
    method!(
        detail,
        "skills/detail",
        SkillsDetailParams,
        SkillsDetailResponse
    );
    method!(
        import,
        "skills/import",
        SkillsImportParams,
        SkillsImportResponse
    );
    method!(
        set_version,
        "skills/setVersion",
        SkillsSetVersionParams,
        SkillsSetVersionResponse
    );
    method!(
        set_latest,
        "skills/setLatest",
        SkillsSetLatestParams,
        SkillsSetLatestResponse
    );
    method!(
        set_alias,
        "skills/setAlias",
        SkillsSetAliasParams,
        SkillsSetAliasResponse
    );
    method!(
        remove,
        "skills/remove",
        SkillsRemoveParams,
        SkillsRemoveResponse
    );
    method!(
        set_enabled,
        "skills/setEnabled",
        SkillsSetEnabledParams,
        SkillsSetEnabledResponse
    );
    method!(
        convert_local,
        "skills/convertLocal",
        SkillsConvertLocalParams,
        SkillsConvertResponse
    );

    async fn call<P: Serialize + Sync, R: DeserializeOwned>(
        &self,
        method: &str,
        params: &P,
    ) -> Result<R, SkillsError> {
        let result = self
            .transport
            .request(method, serde_json::to_value(params)?)
            .await?;
        Ok(serde_json::from_value(result)?)
    }
}
