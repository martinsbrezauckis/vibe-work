//! Typed Account key metadata for command clients, generated to TypeScript.
//! Depends on vw-keys; no credential value can be represented in either view.

use serde::{Deserialize, Serialize};

/// The latest completed test of a stored Mistral key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MistralKeyTestView {
    /// Number of available models on success.
    pub model_count: Option<u32>,
    /// HTTP failure status, when an answer arrived.
    pub code: Option<u16>,
    /// Fixed explanation on failure; never a response body.
    pub text: Option<String>,
}

/// One saved credential without its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MistralKeyView {
    /// Vault item identity.
    pub item_id: String,
    /// Vault display label.
    pub label: String,
    /// This credential is selected.
    pub in_use: bool,
    /// Latest completed test, or null before the first test.
    pub last_test: Option<MistralKeyTestView>,
}

impl From<vw_keys::MistralKey> for MistralKeyView {
    fn from(key: vw_keys::MistralKey) -> Self {
        Self {
            item_id: key.item_id,
            label: key.label,
            in_use: key.in_use,
            last_test: key.last_test.map(|test| MistralKeyTestView {
                model_count: test.model_count,
                code: test.code,
                text: test.text,
            }),
        }
    }
}

pub(crate) async fn execute(
    keys: &vw_keys::KeyService,
    request: crate::KeyRequest,
) -> Result<crate::CommandResult, crate::CoreError> {
    use crate::{CommandResult, CoreError, ErrorCode, KeyRequest};
    let done = Ok(CommandResult::Done {});
    let map = |e: vw_keys::KeyError| crate::keys::key_error(&e);
    match request {
        KeyRequest::ListMistralKeys {} => Ok(CommandResult::MistralKeys {
            keys: keys
                .mistral_keys()
                .await
                .map_err(map)?
                .into_iter()
                .map(Into::into)
                .collect(),
        }),
        KeyRequest::AddMistralKey { name, api_key } => {
            let key = api_key.into_secret();
            let kept = key.clone();
            keys.add_mistral_key(name, key).await.map_err(map)?;
            kept.register_as(vw_secrets::SecretKind::MistralKey);
            done
        }
        KeyRequest::UseMistralKey { item_id } => {
            keys.use_mistral_key(item_id).await.map_err(map).and(done)
        }
        KeyRequest::RenameMistralKey { item_id, name } => keys
            .rename_mistral_key(item_id, name)
            .await
            .map_err(map)
            .and(done),
        KeyRequest::RemoveMistralKey { item_id } => keys
            .remove_mistral_key(item_id)
            .await
            .map_err(map)
            .and(done),
        _ => Err(CoreError::new(
            ErrorCode::Internal,
            "Unexpected Mistral key operation.",
        )),
    }
}
