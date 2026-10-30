//! The New chat part of the command API: the requests the window sends inside `Command::NewChat` and the
//! view that comes back. The New chat screen is a working place before the first message; its terminals and
//! browser belong to an owner id the core makes (not a Vibe session) until the first message hands them over.
//!
//! Depends on: serde. Called by: `command`, `new_chat` (the handler), the UI through the generated TypeScript,
//! vw-debug-api (`POST /commands`).
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - Nothing here reaches Vibe: the owner id is never sent to it.

use serde::{Deserialize, Serialize};

/// One request about the New chat screen's open things.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NewChatRequest {
    /// The current owner id and folder.
    State {},
    /// The folder chosen on the New chat screen (none: new terminals open in the home folder). Running
    /// terminals keep the folder they started in.
    SetFolder {
        /// An absolute folder the user chose, a session works in, or a project's main folder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        folder: Option<String>,
    },
    /// The first message started `sessionId`: its terminals, browser (with the App preview's site data, so its
    /// sign-ins stay) and port lease move to that session (nothing stops), and the New chat screen gets a fresh owner
    /// id.
    Adopt {
        /// The session the first message started.
        session_id: String,
    },
    /// Closes everything open before the chat: the terminals stop, the browser pages close and their local
    /// site data is cleared, and the New chat screen gets a fresh owner id. The folder stays.
    CloseAll {},
}

/// The New chat screen as the core knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct NewChatView {
    /// The id terminal, preview and browser requests use for the New chat screen (internal, never shown).
    pub owner_id: String,
    /// The folder new terminals open in; `None` with `home` set: the home folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub folder: Option<String>,
    /// The home folder new terminals open in when no folder is chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub home: Option<String>,
}
