//! The data the Changes tab shows and the requests it sends back: changed files, hunks and their lines,
//! and what a keep or revert targets.
//!
//! Depends on: serde (wire form, camelCase), ts-rs behind the `ts` feature (TypeScript for the UI through
//! vw-core-api's generator). Called by: `diff` and `parse` (build them), `review` (reads targets),
//! vw-core-api (views and commands).
//! Invariants:
//! - A hunk's position in both versions (what keep/revert splice by) is never serialized: clients see
//!   display lines and an id; the core finds the hunk again by id in a fresh diff.
//! - Display text is lossy UTF-8 with a trailing `\r` removed (CRLF files show no `^M`); the bytes keep it.
//! - Counts are `u32` (a file with more changed lines than that is `tooLarge` long before).

use serde::{Deserialize, Serialize};

/// How a file differs from the baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum FileStatus {
    /// Not in the baseline: the file did not exist when the review started (untracked files that existed
    /// then are part of the baseline).
    Added,
    /// Content (or mode) changed.
    Modified,
    /// In the baseline, gone now.
    Deleted,
    /// Moved (with or without content changes); `old_path` names the source.
    Renamed,
    /// The kind changed (file <-> symlink <-> submodule).
    TypeChanged,
}

/// What kind of content the file has, which decides what can be done per hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ContentKind {
    /// Text with hunks.
    Text,
    /// Binary: counts unknown, file-level actions only.
    Binary,
    /// Text with too many changed lines (or too many bytes) to show here: real counts and hunk count, no
    /// lines, file-level actions only; never accepted by "keep all".
    TooLarge,
    /// A submodule (gitlink): listed, no actions (review it in its own repository).
    Submodule,
    /// An untracked file too big to hash into the repository: listed, not reviewed, no actions.
    LargeUntracked,
}

/// One line of a hunk, numbered like the inline diff (`ui/src/features/transcript/model/diff.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    /// `ctx`, `add` or `del` (the same names as the transcript diff's `DiffLine.kind`).
    pub kind: LineKind,
    /// Line number in the baseline version, when the line is there.
    pub old_no: Option<u32>,
    /// Line number in the current version, when the line is there.
    pub new_no: Option<u32>,
    /// The line without its end-of-line characters.
    pub text: String,
    /// The line has no newline at the end of the file (git's `\ No newline at end of file`).
    pub no_eol: bool,
}

/// Kind of a diff line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum LineKind {
    /// Unchanged context.
    Ctx,
    /// Only in the current version.
    Add,
    /// Only in the baseline.
    Del,
}

/// One hunk: a contiguous change with its context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    /// Stable id within one diff: a hash of the path and the hunk's exact bytes. Actions name it, and a hunk
    /// whose bytes changed has a new id (so an action on the old one is refused as stale).
    pub id: String,
    /// First baseline line (1-based; 0 for an insertion at the top).
    pub old_start: u32,
    /// Baseline lines covered.
    pub old_lines: u32,
    /// First current line.
    pub new_start: u32,
    /// Current lines covered.
    pub new_lines: u32,
    /// The function/section text git prints after `@@ ... @@` (may be empty).
    pub section: String,
    /// The lines.
    pub lines: Vec<DiffLine>,
    /// 0-based first baseline line (kept by the core, never sent).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub old_at: u32,
    /// 0-based first current line (kept by the core, never sent).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub new_at: u32,
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// Path relative to the repository root, `/`-separated, as shown: bytes that are not UTF-8 appear as
    /// `\xNN` and a backslash as `\\` (one-to-one, so every file has its own name here).
    pub path: String,
    /// The baseline path of a rename.
    pub old_path: Option<String>,
    /// How it changed.
    pub status: FileStatus,
    /// What it contains.
    pub content: ContentKind,
    /// Added lines (0 when unknown, e.g. binary).
    pub added: u32,
    /// Removed lines.
    pub removed: u32,
    /// Baseline mode when it differs from the current one (`100644`, `100755`, `120000`, ...).
    pub old_mode: Option<String>,
    /// Current mode when it differs from the baseline one.
    pub new_mode: Option<String>,
    /// The current blob id (`null` for a deleted file): actions carry it back to prove the user saw this
    /// version.
    pub blob: Option<String>,
    /// Size in bytes, for `largeUntracked` files.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub size: Option<u64>,
    /// The hunks (empty unless `content` is `text`).
    pub hunks: Vec<Hunk>,
    /// How many hunks the change has, also when they are not shown (`tooLarge`); 0 when not compared
    /// (binary, or bigger than the comparison limit).
    pub hunk_count: u32,
    /// Which turn changed it, when known (filled by the core).
    pub turn: Option<TurnLabel>,
    /// A rename whose two names differ only in case, on a case-insensitive file system (Windows, macOS):
    /// both names are one file, so a revert renames it back and deletes nothing.
    #[serde(default)]
    pub case_only: bool,
    /// The current path as git's bytes (`path` is its shown form; kept by the core, never sent).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub raw_path: Vec<u8>,
    /// The baseline path of a rename as git's bytes (kept by the core, never sent).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub raw_old_path: Option<Vec<u8>>,
    /// The baseline blob id (kept by the core, never sent).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub base_id: Option<String>,
    /// The baseline mode (kept by the core, never sent).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub base_mode: Option<String>,
    /// The current mode (kept by the core, never sent).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub mode: Option<String>,
}

/// The turn a file change is attributed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TurnLabel {
    /// 1-based turn number in the session.
    pub number: u32,
    /// Vibe's turn id.
    pub turn_id: String,
    /// `effect`: a completed file edit or write in that turn names the file; `observed`: the file changed
    /// while that turn was running (e.g. through a command).
    pub source: TurnSource,
}

/// Where a turn label comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum TurnSource {
    /// A completed `file_write`/`file_edit` effect of that turn names the file.
    Effect,
    /// A refresh saw the file change while that turn ran.
    Observed,
}

/// The diff of a working tree against a baseline.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    /// Changed files, in git's path order.
    pub files: Vec<FileChange>,
    /// A hash over every file's path, status and blob: a bulk action carries it back.
    pub fingerprint: String,
    /// Things the user should know about this diff (untracked files skipped,...).
    pub notes: Vec<String>,
}

impl Diff {
    /// The file at `path` (current path, or the old path of a rename).
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&FileChange> {
        self.files
            .iter()
            .find(|f| f.path == path || f.old_path.as_deref() == Some(path))
    }
}

/// What a keep or revert acts on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Target {
    /// Every changed file. `fingerprint` is the diff's fingerprint the user saw.
    All {
        /// `Diff::fingerprint` as shown.
        fingerprint: String,
    },
    /// One file. `blob` is its current blob id as shown (`null` for a deleted file).
    File {
        /// Current path (or the old path of a rename).
        path: String,
        /// `FileChange::blob` as shown.
        blob: Option<String>,
    },
    /// One hunk of one text file.
    Hunk {
        /// Current path.
        path: String,
        /// `FileChange::blob` as shown.
        blob: Option<String>,
        /// `Hunk::id` as shown.
        hunk_id: String,
    },
}
