//! The `search` command's request and answer types. Results open at the matching conversation
//! message or file line, with each result showing its path and search scope.
//!
//! Depends on: serde, ts-rs (feature `ts`). Called by: `command`, `search`, `search_index`, `search_files`, the UI
//! (generated `sessions.ts`), the Debug API (`POST /commands`).
//! Invariants: wire form `{"op": "<camelCase>", ...}` with unknown fields refused; a query is matched word by word
//! (every word must occur, case-insensitive), never as a pattern.

use serde::{Deserialize, Serialize};

/// What to search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SearchRequest {
    /// The text of every conversation's messages (yours and the agent's), from the core's index. The first search
    /// starts filling the index from Vibe's stored history (the answer's `index` says how far it is); later
    /// searches see changes once a session's `updatedAt` moves.
    Messages {
        /// The words to find.
        query: String,
        /// Only this session (a conversation's own search); every session when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
        /// Most hits returned (default 50, at most 200).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        limit: Option<u32>,
    },
    /// File contents and names in the sessions' folders (or one session's folders): lines that hold every word.
    /// Ignored files of a git repository and build/dependency folders are skipped; a file over 1 MiB or with
    /// binary content is not read.
    Files {
        /// The words to find.
        query: String,
        /// Match only file names, without reading content; an empty query lists names.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        names_only: Option<bool>,
        /// One exact known session folder or user-picked folder (also before a new chat starts).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        folder: Option<String>,
        /// Only this session's folders; the folders of every listed session when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
        /// Most hits returned (default 50, at most 200).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        limit: Option<u32>,
    },
    /// How far the message index is.
    Status {},
}

/// One message that holds the words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MessageHit {
    /// The session.
    pub session_id: String,
    /// Its title (Vibe's), if any.
    pub title: Option<String>,
    /// Its folder.
    pub cwd: Option<String>,
    /// The history entry (the transcript row carries it as `data-entry-id`).
    pub entry_id: String,
    /// `user` or `assistant`.
    pub role: String,
    /// The turn number in the session (1 = the first message you sent).
    pub turn: u32,
    /// The text around the first match, on one line (at most about 160 characters, `…` where cut).
    pub snippet: String,
    /// When the message was written (epoch ms), when known.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub at: Option<u64>,
    /// Every word was found as one phrase (ranked first).
    pub phrase: bool,
}

/// How far the message index is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    /// Sessions whose messages are in the index.
    pub indexed: u32,
    /// Sessions known (listed by Vibe, archived included).
    pub total: u32,
    /// The index is being filled or refreshed now (search again for more).
    pub filling: bool,
    /// The last fill failed (the message), if it did.
    pub error: Option<String>,
}

/// One file line (or file name) that holds the words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FileHit {
    /// The searched folder that holds it (a session's folder).
    pub folder: String,
    /// The path relative to `folder`, with `/` separators.
    pub path: String,
    /// The line (1-based); `null` when the file's name matched.
    pub line: Option<u32>,
    /// The line's text around the match (at most about 160 characters), or the path for a name match.
    pub text: String,
    /// A session that works in `folder` (the newest), to open the file in.
    pub session_id: Option<String>,
}

/// The answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SearchResult {
    /// `messages`.
    Messages {
        /// Best first: phrase matches, then newer sessions, then newer messages.
        hits: Vec<MessageHit>,
        /// Hits found in all (counted up to 1,000).
        total: u32,
        /// How far the index is.
        index: IndexStatus,
    },
    /// `files`.
    Files {
        /// Name matches first, then lines in file order.
        hits: Vec<FileHit>,
        /// Hits found in all (counted up to 1,000).
        total: u32,
        /// The folders searched.
        folders: Vec<String>,
        /// Files read.
        scanned: u32,
        /// The search stopped early (time, file or size budget); there may be more.
        truncated: bool,
    },
    /// `status`.
    Status {
        /// How far the index is.
        index: IndexStatus,
    },
}
