//! The key service's own settings file: which vault item holds the key (Vibe Work's resource, or an
//! existing Mistral item the user chose). No secret is ever written here.
//!
//! Depends on: `serde_json`. Called by: `service`.

use std::path::PathBuf;

use crate::error::KeyError;

/// The settings file.
pub struct KeySettings {
    path: PathBuf,
}

impl KeySettings {
    /// Settings at `path` (read on demand).
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The item holding the key (default: Vibe Work's own resource).
    #[must_use]
    pub fn key_item_id(&self) -> String {
        std::fs::read(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
            .and_then(|v| {
                v.get("keyItemId")
                    .and_then(|id| id.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| vw_vault::mistral_key::RESOURCE_ID.to_owned())
    }

    /// Saves the item id (atomic replace).
    ///
    /// # Errors
    /// [`KeyError::Settings`].
    pub fn set_key_item_id(&self, item_id: &str) -> Result<(), KeyError> {
        let body = serde_json::json!({"keyItemId": item_id}).to_string();
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| KeyError::Settings(e.to_string()))?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, body)
            .and_then(|()| std::fs::rename(&tmp, &self.path))
            .map_err(|e| KeyError::Settings(e.to_string()))
    }
}
