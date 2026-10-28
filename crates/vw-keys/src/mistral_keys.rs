//! Named Mistral credentials and their nonsecret test history, owned by the key service.
//! Called by the Account commands; vault values stay inside the service and the model client.

use serde::{Deserialize, Serialize};
use vw_vault::items::VaultItem;

use super::{
    KeyError, KeyService, SecretString, TestCallError, TestCallOk, blocking, mistral_key, now_ms,
    valid_key,
};

const TEST_FIELD: &str = "vibeWorkModelKeyTest";

/// Last completed test of one stored credential. No response body or credential value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredKeyTest {
    /// Model count on success.
    pub model_count: Option<u32>,
    /// HTTP failure status, when Mistral answered.
    pub code: Option<u16>,
    /// Fixed test explanation on failure.
    pub text: Option<String>,
}

impl StoredKeyTest {
    fn of(result: &Result<TestCallOk, TestCallError>) -> Self {
        match result {
            Ok(ok) => Self {
                model_count: Some(u32::try_from(ok.model_count).unwrap_or(u32::MAX)),
                code: None,
                text: None,
            },
            Err(error) => Self {
                model_count: None,
                code: match error {
                    TestCallError::Unauthorized => Some(401),
                    TestCallError::Forbidden => Some(403),
                    TestCallError::RateLimited => Some(429),
                    TestCallError::Server(code) | TestCallError::Unexpected(code) => Some(*code),
                    TestCallError::Network(_) | TestCallError::MalformedKey => None,
                },
                text: Some(error.to_string()),
            },
        }
    }
}

/// One Account row; this type cannot carry the stored value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MistralKey {
    /// Vault identity for subsequent user actions.
    pub item_id: String,
    /// Item display label.
    pub label: String,
    /// This is the selected credential.
    pub in_use: bool,
    /// Last completed test, if any.
    pub last_test: Option<StoredKeyTest>,
}

fn candidate(item: &VaultItem) -> bool {
    !mistral_key::find_all(std::slice::from_ref(item)).is_empty()
}

impl KeyService {
    /// Lists all Mistral credentials without returning values.
    ///
    /// # Errors
    /// Vault locked or unreadable.
    pub async fn mistral_keys(&self) -> Result<Vec<MistralKey>, KeyError> {
        let vault = self.vault();
        let selected = self.settings.key_item_id();
        blocking(move || {
            let mut items = vault.list_items_strict()?;
            items.sort_by(|a, b| {
                a.created_ms
                    .cmp(&b.created_ms)
                    .then_with(|| a.id.cmp(&b.id))
            });
            Ok(mistral_key::find_all(&items)
                .into_iter()
                .map(|c| {
                    let last_test = items
                        .iter()
                        .find(|i| i.id == c.item_id)
                        .and_then(|i| i.extra.get(TEST_FIELD))
                        .and_then(|v| serde_json::from_value(v.clone()).ok());
                    MistralKey {
                        in_use: c.item_id == selected,
                        item_id: c.item_id,
                        label: c.label,
                        last_test,
                    }
                })
                .collect())
        })
        .await?
    }

    /// Stores a separate, named credential, tests it, and leaves the selection alone.
    /// Failed tests stay on the row so the user can remove the item.
    ///
    /// # Errors
    /// Invalid input or vault storage errors.
    pub async fn add_mistral_key(&self, name: String, key: SecretString) -> Result<(), KeyError> {
        let name = key_name(&name)?;
        if !valid_key(&key) {
            return Err(KeyError::InvalidKey);
        }
        let id = mistral_key::new_id();
        let (vault, stored, item_id) = (self.vault(), key.clone(), id.clone());
        blocking(move || {
            let mut resource = mistral_key::resource(&stored, None, now_ms());
            resource.id = item_id;
            resource.label = name;
            vault.upsert_item(&resource.to_item()?)
        })
        .await??;
        let result =
            vw_mistral::list_models(&self.client.get(), &self.test_api_base, key.expose()).await;
        self.record_model_test(id, key.expose().to_owned(), &result)
            .await?;
        self.changed();
        Ok(())
    }

    pub(super) async fn record_model_test(
        &self,
        id: String,
        tested: String,
        result: &Result<TestCallOk, TestCallError>,
    ) -> Result<(), KeyError> {
        let tested = SecretString::new(tested);
        let value =
            serde_json::to_value(StoredKeyTest::of(result)).map_err(|_| KeyError::Internal)?;
        let vault = self.vault();
        blocking(move || {
            vault.update_item(&id, |item| {
                let mut item = item?;
                // A late test must not resurrect a deleted item or describe its replacement.
                if mistral_key::key_in_item(&item).is_none_or(|key| key.expose() != tested.expose())
                {
                    return None;
                }
                item.extra.insert(TEST_FIELD.to_owned(), value.clone());
                Some(item)
            })
        })
        .await??;
        self.changed();
        Ok(())
    }

    /// Selects a saved key and refreshes the model proxy; emits one runtime reset.
    /// The core checks busy chats and asks the native confirmation before calling this.
    ///
    /// # Errors
    /// Missing credential, locked vault or settings write failure.
    pub async fn use_mistral_key(&self, item_id: String) -> Result<(), KeyError> {
        let _selection = self.selection.lock().await;
        if !self
            .mistral_keys()
            .await?
            .iter()
            .any(|k| k.item_id == item_id)
        {
            return Err(KeyError::NotACandidate);
        }
        if self.settings.key_item_id() == item_id {
            return Ok(());
        }
        self.settings.set_key_item_id(&item_id)?;
        self.refresh_key().await;
        self.key_changed();
        Ok(())
    }

    /// Changes only the vault item's display label.
    ///
    /// # Errors
    /// Invalid name, absent credential or vault write error.
    pub async fn rename_mistral_key(&self, item_id: String, name: String) -> Result<(), KeyError> {
        let name = key_name(&name)?;
        let vault = self.vault();
        let result = blocking(move || {
            vault.update_item(&item_id, |item| {
                let mut item = item.filter(candidate)?;
                item.title.clone_from(&name);
                if item.kind == "resource" {
                    item.extra
                        .insert("label".to_owned(), serde_json::Value::String(name.clone()));
                }
                item.updated_ms = now_ms();
                Some(item)
            })
        })
        .await??;
        if result.as_ref().is_none_or(|item| !candidate(item)) {
            return Err(KeyError::NotACandidate);
        }
        self.changed();
        Ok(())
    }

    /// Removes an unused Mistral credential, after the window's destructive dialog.
    ///
    /// # Errors
    /// The item is selected, absent, not a Mistral key, or vault write failed.
    pub async fn remove_mistral_key(&self, item_id: String) -> Result<(), KeyError> {
        let _selection = self.selection.lock().await;
        if self.settings.key_item_id() == item_id {
            return Err(KeyError::KeyInUse);
        }
        let vault = self.vault();
        if blocking(move || vault.delete_item_when(&item_id, candidate)).await?? != Some(true) {
            return Err(KeyError::NotACandidate);
        }
        self.changed();
        Ok(())
    }
}

fn key_name(name: &str) -> Result<String, KeyError> {
    let name = name.trim();
    if !vw_vault::project_secret::valid_name(name) {
        return Err(KeyError::InvalidSecret(
            "Choose a name of 1 to 200 characters without control characters.",
        ));
    }
    Ok(name.to_owned())
}
