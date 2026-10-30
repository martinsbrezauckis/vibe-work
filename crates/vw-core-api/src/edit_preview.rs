//! Edit previews: an approval card for a file edit or write shows the lines it changes with their line numbers in
//! the file, as a unified diff (old and new numbers, 3 lines of context, hunks split by gaps).
//!
//! The preview is computed once, when the card is first seen (the `callback/call` request or the first event or
//! snapshot that carries the open card), from the file on disk at that moment, and kept per (session, card) so every
//! later snapshot and event shows the same preview. It is display-only data on the card's effect
//! (`detail.effect.vwPreview`); Vibe never sees it and the answer to the card does not depend on it.
//!
//! Which files: the path is the agent's, so it is resolved through the File tab's scope (`files_scope`): the
//! session's own folder and the folders the user picked, canonical containment (a link leaving the folder is
//! refused), no UNC or device forms, regular files only. A path outside is refused before anything touches the disk,
//! and a refused path gets no preview at all. A write that creates a file is previewed from its content when its path
//! lies in an allowed folder and its parent resolves inside it. Every preview line passes the vault-value
//! withholding chat titles use (`title_guard`).
//!
//! An edit is replayed on the file's text the way Vibe applies it: each change in order, its `oldString` once (it must
//! occur exactly once) or, with `replaceAll`, every occurrence, all compared with LF line ends (a CRLF file takes a
//! change written with LF). A write is compared with the file it replaces. When the file cannot be read as text, is
//! larger than [`MAX_FILE`], or a change does not apply, no preview is attached and the card keeps showing the
//! requested change without file line numbers.
//!
//! Bounds: files up to [`MAX_FILE`], at most [`MAX_OCCURRENCES`] replacements, [`MAX_LINES`] preview lines of at
//! most [`MAX_LINE_CHARS`] characters each, a [`DIFF_TIME`] budget for the diff, [`MAX_KEPT`] previews kept. A
//! bounded preview says what it left out: `more` (lines), `cutChars` (characters of cut lines) and a line's own `cut`,
//! so the card can mark it and offer the whole requested change.
//!
//! Depends on: `similar` (line diff with a deadline), `files_scope`, `names` (the session's folder), the gate (picked
//! folders), `title_guard`. Called by: `pump` (the callback request and events), `sessions` and `subscription`
//! (snapshots), tests ([`preview`]).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use similar::{ChangeTag, TextDiff};

use crate::changes_facts::path_key;
use crate::files_scope::{RealFs, Scope, ScopeFs};

/// Largest file previewed (bytes).
pub const MAX_FILE: u64 = 4 << 20;
/// Most replacements one edit may make and still be previewed.
const MAX_OCCURRENCES: usize = 1024;
/// Most preview lines kept on a card.
pub const MAX_LINES: usize = 500;
/// Longest preview line kept (characters).
pub const MAX_LINE_CHARS: usize = 500;
/// Characters past [`MAX_LINE_CHARS`] still given to the vault redaction, so a value crossing the cut is withheld.
const WITHHOLD_MARGIN: usize = 512;
/// Time budget for the line diff.
const DIFF_TIME: Duration = Duration::from_millis(200);
/// Unchanged lines shown around each change.
const CONTEXT: usize = 3;
/// Previews kept at most (open cards are few; older ones have run).
const MAX_KEPT: usize = 64;

/// The previews computed so far, oldest first: (session, card id, preview or `Null` when none could be made).
#[derive(Default)]
pub(crate) struct Previews(Mutex<VecDeque<(String, String, Value)>>);

/// Makes the preview of one effect for one session (`None`: no preview).
type Make<'a> = &'a dyn Fn(&str, &Value) -> Option<Value>;

impl Previews {
    /// Attaches the preview to every open file edit or write card in `value` (any depth), making it with `make` the
    /// first time a card is seen.
    pub(crate) fn project(&self, value: &mut Value, make: Make<'_>) {
        if value["type"] == "callback" {
            self.attach(value, make);
        }
        match value {
            Value::Object(map) => map.values_mut().for_each(|v| self.project(v, make)),
            Value::Array(list) => list.iter_mut().for_each(|v| self.project(v, make)),
            _ => {}
        }
    }

    fn attach(&self, card: &mut Value, make: Make<'_>) {
        let kind = card.pointer("/detail/effect/kind").and_then(Value::as_str);
        if !matches!(kind, Some("file_edit" | "file_write")) {
            return;
        }
        let session = card["sessionId"].as_str().unwrap_or_default().to_owned();
        let Some(id) = card["callbackId"]
            .as_str()
            .or_else(|| card["id"].as_str())
            .map(str::to_owned)
        else {
            return;
        };
        let kept = {
            let items = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            items
                .iter()
                .find(|(s, c, _)| *s == session && *c == id)
                .map(|(_, _, preview)| preview.clone())
        };
        let preview = if let Some(preview) = kept {
            preview
        } else {
            // Only an open card is previewed: a settled one's file already holds the result.
            let open = card
                .pointer("/state/status")
                .and_then(Value::as_str)
                .is_none_or(|status| status == "open");
            if !open {
                return;
            }
            let effect = card.pointer("/detail/effect").cloned().unwrap_or_default();
            let preview = make(&session, &effect).unwrap_or(Value::Null);
            let mut items = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            items.push_back((session, id, preview.clone()));
            while items.len() > MAX_KEPT {
                items.pop_front();
            }
            preview
        };
        if !preview.is_null()
            && let Some(effect) = card.pointer_mut("/detail/effect")
        {
            effect["vwPreview"] = preview;
        }
    }
}

/// Attaches previews in `value` (see [`Previews::project`]) for the folder the session's agent works in and the
/// picked folders (no `~`), with vault values withheld. A session without a known folder gets none.
pub(crate) fn project(shared: &crate::core::Shared, value: &mut Value) {
    shared.edit_previews.project(value, &|session, effect| {
        // The folder the agent works in, and nothing else for relative paths: Vibe writes a relative path there, so
        // the card must compare with that file (a fallback to the project's main folder would show another file).
        let cwd = shared.names.cwd_of(&shared.store, session)?;
        let scope = Scope {
            session: PathBuf::from(cwd),
            cwd: None,
            picked: shared.gate.picked_roots(),
            home: None,
            hidden: true,
        };
        preview(effect, &scope, &RealFs, &|text| {
            crate::title_guard::withhold_text(shared, text)
        })
    });
}

/// Where a preview reads from.
enum Target {
    /// An existing regular file the scope allows (canonical path).
    File(PathBuf),
    /// No file yet, at a path in an allowed folder whose parent resolves inside it: a write creates it.
    New,
}

/// The target of `text` for `scope`, or `None` when the scope refuses it. The disk is touched only for a path whose
/// lexical form lies in an allowed folder (the `files_scope` rule).
fn target(scope: &Scope, text: &str, fs: &dyn ScopeFs, may_create: bool) -> Option<Target> {
    if let Some(hit) = crate::files_scope::resolve_in(scope, text, fs) {
        return (hit.size <= MAX_FILE).then_some(Target::File(hit.canonical));
    }
    if !may_create {
        return None;
    }
    let path = crate::files_scope::candidate(scope, text)?;
    let session = crate::files_scope::lexical(&scope.session)?;
    let session_canonical = fs.canonicalize(&session).ok();
    let roots: Vec<PathBuf> = std::iter::once(session)
        .chain(session_canonical)
        .chain(scope.picked.iter().cloned())
        .collect();
    let inside = |p: &Path| {
        roots
            .iter()
            .any(|root| path_key(p).starts_with(path_key(root)))
    };
    // Lexical gate first: nothing below touches the disk for a path outside every allowed folder.
    if !inside(&path) {
        return None;
    }
    let parent = fs.canonicalize(path.parent()?).ok()?;
    if !inside(&parent) {
        return None;
    }
    match fs.facts(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(Target::New),
        _ => None,
    }
}

/// The preview of one file edit or write `effect` for `scope`, or `None` when it cannot be made or the scope refuses
/// the path (see the module header). `fs` is the filesystem the scope checks go through; the file itself is read only
/// after they allowed it. `withhold` is applied to every preview line's text.
#[must_use]
pub fn preview(
    effect: &Value,
    scope: &Scope,
    fs: &dyn ScopeFs,
    withhold: &dyn Fn(&str) -> String,
) -> Option<Value> {
    let input = effect.get("input")?;
    let path = input.get("filePath")?.as_str()?;
    let kind = effect.get("kind")?.as_str()?;
    if !matches!(kind, "file_edit" | "file_write") {
        return None;
    }
    let target = target(scope, path, fs, kind == "file_write")?;
    let shown = if kind == "file_write" {
        let content = lf(input.get("content")?.as_str()?);
        match target {
            Target::File(file) => render(&lf(&read_text(&file)?), &content, false, withhold),
            Target::New => render("", &content, true, withhold),
        }
    } else {
        let Target::File(file) = target else {
            return None;
        };
        let before = lf(&read_text(&file)?);
        let changes: Vec<&Value> = match input.get("changes").and_then(Value::as_array) {
            Some(list) if !list.is_empty() => list.iter().collect(),
            _ => vec![input],
        };
        let after = apply(&before, &changes)?;
        render(&before, &after, false, withhold)
    };
    Some(shown)
}

/// Opens `path` for reading without following a final link and without waiting on a pipe or device (Unix).
#[cfg(unix)]
fn open(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(i32::try_from(flags.bits()).unwrap_or(0))
        .open(path)
}

/// Opens `path` for reading (the scope check already refused links leaving the folder).
#[cfg(not(unix))]
fn open(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// True when the open `file` is the one now at `path` (Unix: same device and inode, `path` not followed).
#[cfg(unix)]
fn same_file(file: &std::fs::Metadata, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).is_ok_and(|at| at.dev() == file.dev() && at.ino() == file.ino())
}

#[cfg(not(unix))]
fn same_file(_file: &std::fs::Metadata, _path: &Path) -> bool {
    true
}

/// The text of an allowed regular file, read once from one open handle: the handle must be a regular file of at most
/// [`MAX_FILE`] bytes and the file now at `path`, and at most `MAX_FILE + 1` bytes are read, so a file that grew or
/// was swapped after the scope check is never read whole or followed. `None` when too large, binary or not UTF-8.
fn read_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE || !same_file(&meta, path) {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_FILE || bytes[..bytes.len().min(8192)].contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Line ends as LF (CRLF and a lone CR), the form Vibe matches an edit in.
fn lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Replays the changes on `text` (in LF form) in order; `None` when one does not apply as Vibe would apply it.
// Vibe's harness (`_sandbox_helper.py` `edit`, `_replace`): the file and both strings are matched with LF line ends;
// an empty `oldString`, an unchanged one, no match, or several matches without `replaceAll` fail the edit.
fn apply(text: &str, changes: &[&Value]) -> Option<String> {
    let mut text = text.to_owned();
    let mut count = 0usize;
    for change in changes {
        let old = lf(change.get("oldString")?.as_str()?);
        let new = lf(change.get("newString")?.as_str()?);
        let all = change.get("replaceAll").and_then(Value::as_bool) == Some(true);
        if old.is_empty() || old == new {
            return None;
        }
        let found = text.matches(old.as_str()).count();
        if found == 0 || (found > 1 && !all) {
            return None;
        }
        count += found;
        if count > MAX_OCCURRENCES {
            return None;
        }
        text = text.replace(old.as_str(), &new);
    }
    Some(text)
}

/// One preview line: kind, old and new file line numbers (1-based, `null` where none applies), the withheld text cut
/// to [`MAX_LINE_CHARS`], and `cut` (characters left out) when it was cut.
fn line(
    kind: &str,
    old: Option<usize>,
    new: Option<usize>,
    text: &str,
    withhold: &dyn Fn(&str) -> String,
) -> (Value, usize) {
    let raw = text.trim_end_matches(['\n', '\r']);
    // Redaction is bounded to the shown part plus a margin, so a vault value that starts inside the shown part is
    // still seen whole: a hostile multi-megabyte line costs no more than a short one.
    let window: String = raw.chars().take(MAX_LINE_CHARS + WITHHOLD_MARGIN).collect();
    let text = withhold(&window);
    let kept: String = text.chars().take(MAX_LINE_CHARS).collect();
    let cut = raw.chars().count().saturating_sub(kept.chars().count());
    let mut value = json!({"kind": kind, "oldNo": old.map(|n| n + 1), "newNo": new.map(|n| n + 1), "text": kept});
    if cut > 0 {
        value["cut"] = json!(cut);
    }
    (value, cut)
}

/// The unified diff of `before` -> `after` as preview lines, bounded to [`MAX_LINES`]: `{lines, more, cutChars,
/// added, removed, newFile}`, where `more` counts the lines left out, `cutChars` the characters cut from kept lines,
/// and `added` / `removed` count every changed line.
fn render(before: &str, after: &str, new_file: bool, withhold: &dyn Fn(&str) -> String) -> Value {
    let diff = TextDiff::configure()
        .deadline(Instant::now() + DIFF_TIME)
        .diff_lines(before, after);
    let mut lines = Vec::new();
    let (mut total, mut added, mut removed, mut cut_chars) = (0usize, 0usize, 0usize, 0usize);
    let mut push = |value: Value, cut: usize| {
        total += 1;
        added += usize::from(value["kind"] == "add");
        removed += usize::from(value["kind"] == "del");
        if lines.len() < MAX_LINES {
            cut_chars += cut;
            lines.push(value);
        }
    };
    let mut kept_lines = 0usize;
    for (index, group) in diff.grouped_ops(CONTEXT).iter().enumerate() {
        if index > 0 {
            kept_lines += 1;
            push(
                json!({"kind": "gap", "oldNo": null, "newNo": null, "text": ""}),
                0,
            );
        }
        for op in group {
            for change in diff.iter_changes(op) {
                let kind = match change.tag() {
                    ChangeTag::Equal => "ctx",
                    ChangeTag::Delete => "del",
                    ChangeTag::Insert => "add",
                };
                // Lines past the card's limit are only counted: never redacted or copied.
                if kept_lines >= MAX_LINES {
                    push(json!({"kind": kind}), 0);
                    continue;
                }
                kept_lines += 1;
                let (value, cut) = line(
                    kind,
                    change.old_index(),
                    change.new_index(),
                    change.value(),
                    withhold,
                );
                push(value, cut);
            }
        }
    }
    let more = total - lines.len();
    json!({"lines": lines, "more": more, "cutChars": cut_chars, "added": added, "removed": removed, "newFile": new_file})
}
