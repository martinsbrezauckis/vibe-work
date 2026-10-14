//! What the Changes tab needs from a session's state: its folder, its turns (number, anchor,
//! running), and the files Vibe's file tools wrote or edited, per turn.
//!
//! Depends on: `serde_json` (the store's wire JSON). Called by: `changes`.
//! Facts used (Vibe 2.26.0, `vibe/app_server/models.py`): `session.cwd`; `session.status = {type: running |
//! blocked, activeTurnId}` (`:1126-1135`); `turns[] = {id, inputEntryId, startedAt, ...}` (`PublicTurn`,
//! `:1232-1241`; `inputEntryId` is the user entry = the rewind anchor, our `clientUserMessageId` on unified);
//! completed effects `detail.kind = file_write | file_edit` with `state.output.filePath` (write) or
//! `state.output.file` (edit), `detail.input.filePath` as fallback, `state.output.fileExisted`
//! (shapes confirmed in protocol captures).
//! Invariants:
//! - Pure reads of the JSON; unknown shapes are skipped, never an error.
//! - Paths from different sources are compared by [`path_key`], never as raw `PathBuf`s: on Windows git
//!   prints `C:/x`, the folder the app picked is canonical `\\?\C:\x` and Vibe reports paths under it
//!   (otherwise every effect label can be lost on Windows 11).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
// The Windows spelling rule lives in vw-platform (the project folders use the same one).
use vw_platform::windows_key;

/// One file write or edit Vibe completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileTouch {
    /// The effect entry id.
    pub(crate) entry_id: String,
    /// Absolute path (relative paths resolved against the session folder).
    pub(crate) path: PathBuf,
    /// The turn it belongs to.
    pub(crate) turn_id: Option<String>,
    /// A `file_write` that created the file.
    pub(crate) created: bool,
}

/// One turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TurnFact {
    /// Vibe's id.
    pub(crate) id: String,
    /// 1-based number in start order.
    pub(crate) number: u32,
    /// The user entry that opened it.
    pub(crate) anchor: Option<String>,
}

/// The session facts the Changes tab uses.
#[derive(Debug, Clone, Default)]
pub(crate) struct Facts {
    /// The session folder.
    pub(crate) cwd: Option<PathBuf>,
    /// Session title.
    pub(crate) title: Option<String>,
    /// The running (or blocked) turn.
    pub(crate) running: Option<String>,
    /// Turns in start order.
    pub(crate) turns: Vec<TurnFact>,
    /// Completed file writes and edits in history order.
    pub(crate) touches: Vec<FileTouch>,
}

impl Facts {
    /// The turn with `id`.
    pub(crate) fn turn(&self, id: &str) -> Option<&TurnFact> {
        self.turns.iter().find(|t| t.id == id)
    }
}

fn text<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

/// Reads the facts from a session's wire state.
pub(crate) fn facts(state: &Value) -> Facts {
    let cwd = text(state, "/session/cwd").map(PathBuf::from);
    let running = match text(state, "/session/status/type") {
        Some("running" | "blocked") => {
            text(state, "/session/status/activeTurnId").map(str::to_owned)
        }
        _ => None,
    };
    let mut turns: Vec<(i64, String, Option<String>)> = state
        .get("turns")
        .and_then(Value::as_array)
        .map(|all| {
            all.iter()
                .filter_map(|t| {
                    let id = text(t, "/id")?.to_owned();
                    let started = t.get("startedAt").and_then(Value::as_i64).unwrap_or(0);
                    Some((started, id, text(t, "/inputEntryId").map(str::to_owned)))
                })
                .collect()
        })
        .unwrap_or_default();
    // Vibe's legacy rewind clears its turn list but keeps earlier history. Those user entries still
    // name the surviving turns and their rewind anchors, so retain their Changes groups.
    if let Some(history) = state.get("history").and_then(Value::as_array) {
        for entry in history {
            if text(entry, "/type") != Some("message") || text(entry, "/role") != Some("user") {
                continue;
            }
            if let (Some(id), Some(anchor)) = (text(entry, "/turnId"), text(entry, "/id"))
                && !turns.iter().any(|(_, known, _)| known == id)
            {
                turns.push((
                    entry.get("createdAt").and_then(Value::as_i64).unwrap_or(0),
                    id.to_owned(),
                    Some(anchor.to_owned()),
                ));
            }
        }
    }
    turns.sort_by_key(|(started, _, _)| *started);
    // A turn without `inputEntryId` (older states, the test server): its user message is the anchor, the
    // same rule as Vibe's `_turn_anchor` (`_unified_harness_snapshots.py`).
    for (_, id, anchor) in &mut turns {
        if anchor.is_none() {
            *anchor = user_entry_of(state, id);
        }
    }
    let turns = turns
        .into_iter()
        .enumerate()
        .map(|(i, (_, id, anchor))| TurnFact {
            id,
            number: u32::try_from(i + 1).unwrap_or(u32::MAX),
            anchor,
        })
        .collect();
    let touches = state
        .get("history")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| touch(e, cwd.as_deref()))
                .collect()
        })
        .unwrap_or_default();
    Facts {
        cwd,
        title: text(state, "/session/title").map(str::to_owned),
        running,
        turns,
        touches,
    }
}

/// The id of the user message that opened `turn_id`.
fn user_entry_of(state: &Value, turn_id: &str) -> Option<String> {
    state
        .get("history")?
        .as_array()?
        .iter()
        .find(|e| {
            text(e, "/type") == Some("message")
                && text(e, "/role") == Some("user")
                && text(e, "/turnId") == Some(turn_id)
        })
        .and_then(|e| text(e, "/id"))
        .map(str::to_owned)
}

fn touch(entry: &Value, cwd: Option<&Path>) -> Option<FileTouch> {
    if text(entry, "/type") != Some("effect") || text(entry, "/state/status") != Some("completed") {
        return None;
    }
    let kind = text(entry, "/detail/kind")?;
    let raw = match kind {
        "file_write" => text(entry, "/state/output/filePath"),
        "file_edit" => text(entry, "/state/output/file"),
        _ => return None,
    }
    .or_else(|| text(entry, "/detail/input/filePath"))?;
    let path = Path::new(raw);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd?.join(path)
    };
    Some(FileTouch {
        entry_id: text(entry, "/id")?.to_owned(),
        path,
        turn_id: text(entry, "/turnId").map(str::to_owned),
        created: kind == "file_write"
            && entry
                .pointer("/state/output/fileExisted")
                .and_then(Value::as_bool)
                == Some(false),
    })
}

/// Per file ([`path_key`] of its absolute path), the last turn whose file effect names it.
pub(crate) fn last_turn_by_path(facts: &Facts) -> HashMap<PathBuf, String> {
    let mut map = HashMap::new();
    for touch in &facts.touches {
        if let Some(turn) = &touch.turn_id {
            map.insert(path_key(&touch.path), turn.clone());
        }
    }
    map
}

/// `path` relative to `base` when inside it (with `/`), else the full path. Inside is decided on
/// [`path_key`]s; the relative part keeps `path`'s own spelling.
pub(crate) fn shown(path: &Path, base: Option<&Path>) -> String {
    let relative = base.and_then(|b| {
        path.strip_prefix(b)
            .map(Path::to_path_buf)
            .ok()
            .or_else(|| {
                let inner = path_key(path)
                    .strip_prefix(path_key(b))
                    .ok()?
                    .components()
                    .count();
                let parts: Vec<_> = path.components().collect();
                let from = parts.len().checked_sub(inner)?;
                Some(parts[from..].iter().collect())
            })
    });
    relative
        .as_deref()
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The form in which paths from different sources are compared: on Windows [`windows_key`] (names compare
/// without case there, and one folder has several spellings: `canonicalize` gives `\\?\C:\x`, a process reports
/// `C:\x`, git prints `C:/x`); elsewhere the path as it is (byte-exact). Public for the tests that compare
/// such paths (`vw_product_launch`), so there is one helper for the whole repository.
#[must_use]
pub fn path_key(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(windows_key(&path.to_string_lossy()))
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::windows_key;

    /// PREDICT (written before `windows_key` existed; Windows 11 showed effect paths `\\?\C:\...\a.txt` against
    /// git's `C:/.../a.txt`): every spelling of one Windows path gives one key.
    #[test]
    fn windows_spellings_of_one_path_share_a_key() {
        let key = windows_key(r"C:\Users\Me\repo\a.txt");
        for same in [
            r"\\?\C:\Users\Me\repo\a.txt",
            "C:/Users/Me/repo/a.txt",
            r"c:\users\me\REPO\a.txt",
            r"C:/Users/Me/repo\a.txt",
        ] {
            assert_eq!(windows_key(same), key, "{same}");
        }
        assert_eq!(
            windows_key(r"\\?\UNC\server\share\x"),
            windows_key(r"\\server\share\x")
        );
        assert_ne!(windows_key(r"C:\repo\a.txt"), windows_key(r"D:\repo\a.txt"));
        assert_ne!(windows_key(r"C:\repo\a.txt"), windows_key(r"C:\repo\b.txt"));
    }
}
