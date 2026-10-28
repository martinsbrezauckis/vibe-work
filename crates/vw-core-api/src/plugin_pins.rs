//! A reopened chat and its plugins. Vibe pins a chat's plugins and keeps its own copy of each (it never deletes
//! those copies), so a chat resumed, continued or forked after the user removed a plugin or turned it off would
//! start that plugin's MCP servers again from the copy. When a pinned plugin's folder is gone, the chat is re-pinned
//! by the core refresh path: it then has the plugins that are installed and approved now.
//!
//! Depends on: `channels` (its parent: routed requests). Called by: `plugin_refresh`, after a successful reopen.

use std::path::Path;

use serde_json::{Value, json};
use tracing::warn;

use super::Channels;

/// True when a plugin pinned in `catalog` (a `plugin_catalog/read` result) has no installed folder any more.
pub(super) fn pinned_folder_gone(catalog: &Value) -> bool {
    catalog
        .pointer("/plugins/plugins")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|pin| match pin.get("installedRoot") {
            Some(Value::String(root)) => !root.is_empty() && !Path::new(root).exists(),
            Some(Value::Null) => {
                pin["scope"]
                    .as_str()
                    .is_some_and(|scope| scope != "builtin")
                    && pin["pinnedRoot"]
                        .as_str()
                        .is_some_and(|root| !root.is_empty())
            }
            _ => false,
        })
}

impl Channels {
    /// Checks a reopened chat's pins; only the core refresh path may change them.
    pub(crate) async fn has_removed_plugins(&self, id: &str) -> bool {
        match self
            .request_routed("plugin_catalog/read", json!({"sessionId": id}))
            .await
        {
            Ok(catalog) => pinned_folder_gone(&catalog),
            Err(error) => {
                warn!(%error, "plugins: the reopened chat's pinned plugins were not read");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_whose_folder_is_gone_is_found() {
        let here = env!("CARGO_MANIFEST_DIR");
        let catalog = |root: &str| json!({"plugins": {"plugins": [{"name": "a", "installedRoot": here}, {"name": "b", "installedRoot": root}]}});
        assert!(pinned_folder_gone(&catalog(
            "/nonexistent/vw-removed-plugin"
        )));
        assert!(!pinned_folder_gone(&catalog(here)));
        assert!(!pinned_folder_gone(&catalog("")));
        assert!(!pinned_folder_gone(&json!({})));
    }
    #[test]
    fn a_pin_without_an_installed_source_is_removed() {
        let pin = |scope: &str| json!({"plugins":{"plugins":[{"name":"review-helper","scope":scope,"pinnedRoot":"/cached/package","installedRoot":null}]}});
        assert!(pinned_folder_gone(&pin("project")));
        assert!(!pinned_folder_gone(&pin("builtin")));
        assert!(!pinned_folder_gone(&json!({"plugins":{"plugins":[{}]}})));
    }
}
