//! Rust-only scan inputs for all Vault secret fields, including hidden resources and website passwords.
//! Depends on Vault item parsers; the core's background scanner calls this blocking read without prompting.

use vw_vault::SecretString;
use vw_vault::items::{ResourceKind, VaultItem, VaultResource};

use crate::{KeyError, KeyService};

/// A scan input. Values never belong in a command, event or incremental record.
#[derive(Debug)]
pub struct VaultScanSecret {
    /// Stable item and field identifier.
    pub id: String,
    /// Display name, with no value or preview.
    pub name: String,
    /// Login identity for opening the existing password replacement form, never a value.
    pub login_id: Option<String>,
    /// Item modification time, used for the set fingerprint.
    pub modified_ms: i64,
    /// Password matches require manual redaction even in automatic mode.
    pub website_password: bool,
    /// Owned, zeroizing value; consumed by the Rust scanner.
    pub value: SecretString,
}

impl KeyService {
    /// Read every Vault secret field without an unlock prompt. Call on a background thread.
    ///
    /// # Errors
    /// A locked Vault or storage failure. No provider request or key-needed event is raised.
    pub fn scan_secrets(&self) -> Result<Vec<VaultScanSecret>, KeyError> {
        Ok(self
            .vault()
            .list_items_strict()?
            .into_iter()
            .flat_map(inputs)
            .collect())
    }
}

fn inputs(mut item: VaultItem) -> Vec<VaultScanSecret> {
    let mut out = Vec::new();
    let resource = VaultResource::from_item(&item);
    let modified_ms = resource.as_ref().map_or(item.updated_ms, |resource| {
        item.updated_ms.max(resource.updated_ms)
    });
    let mut add = |field: &str, name: &str, password: bool, value: String| {
        if value.is_empty() {
            return;
        }
        let value = SecretString::exact(value);
        value.register_as(vw_secrets::SecretKind::VaultValue);
        out.push(VaultScanSecret {
            id: format!("{}:{field}", item.id),
            name: name.to_owned(),
            login_id: password.then(|| item.id.clone()),
            modified_ms,
            website_password: password,
            value,
        });
    };
    if let Some(resource) = resource {
        let login = resource.kind == ResourceKind::Login;
        for (field, value) in resource.secret_fields {
            add(&field, &resource.label, login, value);
        }
        for field in resource
            .custom_fields
            .into_iter()
            .filter(|field| field.concealed)
        {
            add(
                &format!("custom:{}", field.id),
                &resource.label,
                login,
                field.value,
            );
        }
    }
    add(
        "password",
        &item.title,
        item.kind == "login",
        std::mem::take(&mut item.password),
    );
    if item.kind == "note" {
        add("notes", &item.title, false, std::mem::take(&mut item.notes));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn includes_hidden_provider_mcp_custom_fields_and_plain_passwords() {
        let make = |id: &str, kind: &str, extra: serde_json::Value| -> VaultItem {
            serde_json::from_value(json!({"id": id, "type": kind, "title": id,
                "password": if kind == "login" { "invented-password" } else { "" }, "updated_ms": 7,
                "secretFields": extra, "kind": "developerCredential", "permission": "userOnly",
                "customFields": [{"id":"extra", "label":"Extra", "kind":"text", "value":"invented-custom", "concealed":true}]})).unwrap()
        };
        let provider = inputs(make(
            "provider",
            "resource",
            json!({"apiKey":"invented-provider"}),
        ));
        let mcp = inputs(make("mcp", "resource", json!({"token":"invented-mcp"})));
        let password = inputs(make("site", "login", json!({})));
        let mut note = make("note", "note", json!({}));
        note.notes = "invented-private-note".to_owned();
        assert_eq!(inputs(note).len(), 1);
        assert_eq!(provider.len(), 2);
        assert_eq!(mcp.len(), 2);
        assert_eq!(password.len(), 1);
        assert!(password[0].website_password);
        assert!(provider.iter().all(|secret| !secret.website_password));
        assert_eq!(provider[0].modified_ms, 7);
        assert!(!format!("{provider:?}").contains("invented-provider"));
    }
}
