//! The Knowledge base part of the command API: the named places the user keeps (a folder on this computer or a
//! synced drive folder, an online-only cloud drive reached through the user's own connector, or one of the user's
//! Mistral Libraries), and the requests that add, rename, re-note and remove them.
//!
//! Depends on: serde. Called by: `command`, `knowledge` (the handler and the agent text), the UI through the
//! generated TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - A place is a pointer only: it grants no permission and nothing is copied or indexed.
//! - Names, notes and locations are the user's text: the UI renders them as text.

use serde::{Deserialize, Serialize};

/// What kind of place a knowledge base entry points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum KnowledgeKind {
    /// A folder on this computer, including a cloud drive folder synced to disk. `location` is its path.
    Folder,
    /// A cloud drive that is not synced to disk. `location` is the name of the user's connector for it.
    Drive,
    /// One of the user's Mistral Libraries. `location` is the library id; `label` is its name on Mistral.
    MistralLibrary,
}

/// One named place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgePlace {
    /// Stable id (made by the app).
    pub id: String,
    /// The name the user uses for it ("our library").
    pub name: String,
    /// What it is.
    pub kind: KnowledgeKind,
    /// Folder path, connector name or library id (see [`KnowledgeKind`]).
    pub location: String,
    /// The library's name on Mistral (Mistral Libraries only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub label: Option<String>,
    /// A short note for the agent ("contracts and offers since 2024").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub note: Option<String>,
}

/// The whole knowledge base.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeView {
    /// Every place, in the order the user added them.
    pub places: Vec<KnowledgePlace>,
}

/// One request about the knowledge base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum KnowledgeRequest {
    /// The saved places.
    List {},
    /// Adds a place.
    Add {
        /// The name the user uses for it.
        name: String,
        /// What it is.
        kind: KnowledgeKind,
        /// Folder path (absolute), connector name or library id.
        location: String,
        /// The library's name on Mistral (Mistral Libraries only).
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        label: Option<String>,
        /// A short note for the agent.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        note: Option<String>,
    },
    /// Gives a place another name.
    Rename {
        /// The place.
        id: String,
        /// The new name.
        name: String,
    },
    /// Gives a place a new note for the agent; an empty note removes it.
    SetNote {
        /// The place.
        id: String,
        /// The new note (empty: no note).
        note: String,
    },
    /// Removes a place from the list (the folder, drive or library itself is not touched).
    Remove {
        /// The place.
        id: String,
    },
}

#[cfg(feature = "ts")]
pub(crate) fn declarations(config: &ts_rs::Config) -> Vec<String> {
    use ts_rs::TS;
    vec![
        KnowledgeKind::decl(config),
        KnowledgePlace::decl(config),
        KnowledgeView::decl(config),
        KnowledgeRequest::decl(config),
    ]
}
