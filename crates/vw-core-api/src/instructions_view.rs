//! The instructions part of the command API: the requests both clients send inside
//! `Command::Instructions` and what comes back. Instructions are the `AGENTS.md` files Vibe puts in
//! the system prompt. This command reads and writes the personal file in Vibe Work's own `VIBE_HOME`.
//!
//! Depends on: serde. Called by: `command`, `instructions` (the handler), the UI through the
//! generated TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - Personal requests always address the file in Vibe Work's own home. Projects shares the document view.
//! - Nothing here names a path inside the user's CLI home beyond what [`InstructionsDoc::cli`]
//!   states (found or not): the CLI file's text only travels in a `readCli` answer.

use serde::{Deserialize, Serialize};

/// Which document a request is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum InstructionsScope {
    /// `$VIBE_HOME/AGENTS.md` of Vibe Work's own home: applies to every session of the app.
    Personal,
}

/// One request about instructions. The Debug API acts with the user's rights; reads and writes
/// from it and the app window run directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum InstructionsRequest {
    /// Reads one document (text, whether it can be edited, where it is).
    Read {
        /// Personal instructions.
        scope: InstructionsScope,
    },
    /// Reads the personal `AGENTS.md` of the user's own Vibe command-line home, so the user can
    /// copy it into Vibe Work's. One-way: nothing is ever written there. From the Debug API origin
    /// this runs with the user's rights, as other Debug API reads do.
    ReadCli {},
    /// Replaces one document with `text`, exactly. Refused when the file no longer holds what the
    /// user saw (`expected`), when it is a link, or when the text is over the size limit.
    Write {
        /// Personal instructions.
        scope: InstructionsScope,
        /// The complete new text.
        text: String,
        /// The text the user saw when the preview was made (absent: the file did not exist).
        /// The write is refused when the file holds something else now.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        expected: Option<String>,
    },
    /// Makes open sessions take the current files. Vibe builds a session's instructions once, when its
    /// runtime is created (observed live, the first turn captures them, `config/reload`
    /// does not refresh them), and rebuilds them when the session is reopened. So this closes the
    /// channels of the affected sessions: a quiet session closes now, a busy one when its turn ends;
    /// the next message reopens the session (`session/resume`) with the new text. No file is touched.
    Apply {
        /// Every open session.
        scope: InstructionsScope,
    },
}

/// One document as the settings screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InstructionsDoc {
    /// The file's full path (shown to the user; the user owns these folders).
    pub path: String,
    /// The folder the project document lives in (`project` only).
    pub folder: Option<String>,
    /// The file exists.
    pub exists: bool,
    /// Its text (empty when it does not exist or cannot be shown).
    pub text: String,
    /// The app may edit it. False for a link, a folder, a file that is not text or a file that is
    /// too large; `note` says why in plain words.
    pub editable: bool,
    /// Why it is not editable, or a short remark.
    pub note: Option<String>,
    /// `personal` only: the user's own Vibe command-line home has a personal `AGENTS.md` that can be
    /// copied. `None` for a project document.
    pub cli_available: Option<bool>,
}

/// What an instructions request returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InstructionsResult {
    /// `read`, or `write` (the document as it is now).
    Doc {
        /// The document.
        doc: InstructionsDoc,
    },
    /// `readCli`.
    Cli {
        /// The CLI home has a non-empty personal `AGENTS.md`.
        available: bool,
        /// Its text (empty when not available).
        text: String,
    },
    /// `apply`.
    Applied {
        /// Sessions that will use the current text from their next message: a session that ran a
        /// turn is closed and reopens; one that never ran a turn is reloaded.
        reopened: u32,
        /// Busy sessions that will be closed when their turn ends.
        waiting: u32,
    },
}
