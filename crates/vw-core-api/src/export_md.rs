//! Renders one session as Markdown: pure functions over
//! Vibe's public history entries (`PublicHistoryEntry`, `vibe/app_server/models.py:959-1082`), no I/O.
//!
//! Shape of the document: a title, a short facts list (folder, model, when, how many messages), then the
//! conversation in order: `## You` / `## Vibe` sections with the message text as written (Vibe's answers are
//! Markdown already), the agent's tool steps summarised as one bullet each ("Ran `npm test` (failed)"), and the
//! file changes of `file_edit` / `file_write` steps as fenced `diff` blocks. Reasoning, tool output and approval
//! internals are left out (the header says so).
//!
//! Depends on: `serde_json`, `similar` (line diffs). Called by: `session_export` (the `exportSession` command).
//! Invariants:
//! - Labels come from Vibe's own display fields (`detail.display`, `state.display`), the same pairing rule as the
//!   transcript (`ui/src/features/transcript/model/entries.ts` `settledLabel`); nothing is invented.
//! - A fenced block is always longer than the longest backtick run inside it, so content cannot close it.
//! - Unknown entry types and kinds never fail the export: they are skipped (a newer Vibe may add some).
//! - Each diff is capped ([`MAX_DIFF_LINES`]); a cut says how many lines were left out.
//! - Secret values are not handled here: the caller scrubs the finished text (`vw_secrets::scrub`).

use std::fmt::Write as _;
use std::time::Duration;

use serde_json::Value;
use similar::TextDiff;

/// Most lines of one diff block before it is cut.
pub(crate) const MAX_DIFF_LINES: usize = 400;

/// Facts for the document's header.
#[derive(Debug, Clone, Default)]
pub(crate) struct ExportHead {
    /// The session's title (Vibe's), if it has one.
    pub title: Option<String>,
    /// The session's folder.
    pub cwd: Option<String>,
    /// The model the session ran with last.
    pub model: Option<String>,
    /// Vibe's session id.
    pub session_id: String,
    /// When the export was made, epoch milliseconds.
    pub exported_at_ms: u64,
}

/// What the renderer counted (the command's answer reports it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ExportCounts {
    /// User and assistant messages written.
    pub messages: u32,
    /// Tool steps summarised.
    pub steps: u32,
    /// Diff blocks written.
    pub diffs: u32,
}

fn s<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

/// `2026-10-10 14:02 UTC` for epoch milliseconds (the civil-from-days algorithm; no date crate needed).
pub(crate) fn utc_minute(ms: u64) -> String {
    let secs = ms / 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Howard Hinnant's days_from_civil inverse.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// A code fence longer than any backtick run in `body` (at least three).
fn fence_for(body: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in body.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}

/// Inline code for a short label (a command, a path): backticks around it, longer than any run inside.
fn inline_code(text: &str) -> String {
    let one_line: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let clipped: String = if one_line.chars().count() > 200 {
        let head: String = one_line.chars().take(200).collect();
        format!("{head}…")
    } else {
        one_line
    };
    let ticks = fence_for(&clipped).len() - 2;
    let marks = "`".repeat(ticks.max(1));
    let pad = if clipped.starts_with('`') || clipped.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{marks}{pad}{clipped}{pad}{marks}")
}

/// The text of a message's text blocks, plus a note per picture or attached document.
fn message_body(entry: &Value) -> String {
    let mut text = String::new();
    let mut notes: Vec<String> = Vec::new();
    for block in entry
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match s(block, "type") {
            Some("text") => text.push_str(s(block, "text").unwrap_or_default()),
            Some("image") => {
                let alias = block
                    .pointer("/attachment/alias")
                    .and_then(Value::as_str)
                    .unwrap_or("picture");
                notes.push(format!("*[picture: {alias}]*"));
            }
            Some("resource") => {
                let name = block
                    .pointer("/resource/name")
                    .or_else(|| block.pointer("/resource/uri"))
                    .and_then(Value::as_str)
                    .unwrap_or("document");
                notes.push(format!("*[attached: {name}]*"));
            }
            _ => {}
        }
    }
    let mut body = text.trim_end().to_owned();
    for note in notes {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str(&note);
    }
    body
}

/// The settled label of an effect, as the transcript shows it (`settledLabel` in `entries.ts`).
fn effect_label(entry: &Value) -> (String, String) {
    let call = entry.pointer("/detail/display").unwrap_or(&Value::Null);
    let settled = entry.pointer("/state/display");
    let settled_verb = settled.and_then(|d| s(d, "verb")).unwrap_or_default();
    let (verb, mut target) = if settled_verb.is_empty() {
        (
            nonempty(s(call, "settledVerb"))
                .or_else(|| s(call, "verb"))
                .unwrap_or_default()
                .to_owned(),
            nonempty(s(call, "settledMessage"))
                .or_else(|| nonempty(s(call, "message")))
                .or_else(|| s(call, "summary"))
                .unwrap_or_default()
                .to_owned(),
        )
    } else {
        (
            settled_verb.to_owned(),
            nonempty(settled.and_then(|d| s(d, "message")))
                .or_else(|| nonempty(s(call, "settledMessage")))
                .or_else(|| s(call, "message"))
                .unwrap_or_default()
                .to_owned(),
        )
    };
    if !verb.is_empty()
        && let Some(rest) = target.strip_prefix(&format!("{verb} "))
    {
        target = rest.to_owned();
    }
    let verb = if verb.is_empty() {
        s(entry, "title").unwrap_or("Step").to_owned()
    } else {
        verb
    };
    (verb, target)
}

/// One bullet for an effect: "Ran `npm test` (failed)".
fn step_line(entry: &Value) -> String {
    let (verb, target) = effect_label(entry);
    let status = entry
        .pointer("/state/status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let outcome = match status {
        "failed" => " (failed)",
        "cancelled" => " (cancelled)",
        "denied" | "rejected" => " (denied)",
        _ if entry.pointer("/state/display/success") == Some(&Value::Bool(false)) => " (failed)",
        _ => "",
    };
    if target.is_empty() {
        format!("- {verb}{outcome}")
    } else {
        format!("- {verb} {}{outcome}", inline_code(&target))
    }
}

/// `text` ending in a newline (so a missing final newline never shows as a change or a "No newline" marker).
fn with_eol(text: &str) -> String {
    if text.is_empty() || text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
    }
}

/// The hunks of `old` -> `new` (3 lines of context), without file headers.
fn hunks(old: &str, new: &str) -> String {
    let (old, new) = (with_eol(old), with_eol(new));
    let diff = TextDiff::configure()
        .timeout(Duration::from_millis(200))
        .diff_lines(&old, &new);
    diff.unified_diff().context_radius(3).to_string()
}

/// `--- a/<path>` / `+++ b/<path>` and the hunks, capped at [`MAX_DIFF_LINES`].
fn unified(path: &str, body: &str) -> String {
    let path = path.replace(['\n', '\r'], " ");
    let text = format!("--- a/{path}\n+++ b/{path}\n{body}");
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= MAX_DIFF_LINES {
        return text.trim_end().to_owned();
    }
    let mut out = lines[..MAX_DIFF_LINES].join("\n");
    let _ = write!(
        out,
        "\n… {} more lines left out",
        lines.len() - MAX_DIFF_LINES
    );
    out
}

/// The diff of a completed `file_edit` (`state.output.occurrences`, one hunk group per occurrence; hunk line numbers
/// count from the edited text, the file's line is in the text after the path) or `file_write`
/// (`state.output.{content, previousContent}`).
fn effect_diff(entry: &Value) -> Option<String> {
    if entry.pointer("/state/status").and_then(Value::as_str) != Some("completed") {
        return None;
    }
    let kind = entry
        .pointer("/detail/kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let output = entry.pointer("/state/output")?;
    let input = entry.pointer("/detail/input");
    let path = s(output, "file")
        .or_else(|| s(output, "filePath"))
        .or_else(|| input.and_then(|i| s(i, "filePath")))
        .unwrap_or("file")
        .to_owned();
    match kind {
        "file_edit" => {
            let parts: Vec<String> = output
                .get("occurrences")
                .and_then(Value::as_array)?
                .iter()
                .map(|o| {
                    hunks(
                        s(o, "oldText").unwrap_or_default(),
                        s(o, "newText").unwrap_or_default(),
                    )
                })
                .collect();
            let first = output
                .pointer("/occurrences/0/startLine")
                .and_then(Value::as_u64);
            let label = first.map_or(path.clone(), |line| format!("{path} (from line {line})"));
            (!parts.is_empty()).then(|| unified(&label, &parts.concat()))
        }
        "file_write" => {
            let content = s(output, "content")?;
            let previous = s(output, "previousContent").unwrap_or_default();
            Some(unified(&path, &hunks(previous, content)))
        }
        _ => None,
    }
}

/// Writes the whole document. `entries` are in history order (oldest first).
pub(crate) fn render(head: &ExportHead, entries: &[Value]) -> (String, ExportCounts) {
    let mut counts = ExportCounts::default();
    let mut out = String::new();
    let title = head
        .title
        .as_deref()
        .and_then(|t| nonempty(Some(t)))
        .unwrap_or("Untitled session");
    let _ = writeln!(out, "# {}\n", title.replace('\n', " "));
    if let Some(cwd) = head.cwd.as_deref() {
        let _ = writeln!(out, "- Folder: {}", inline_code(cwd));
    }
    if let Some(model) = head.model.as_deref() {
        let _ = writeln!(out, "- Model: {}", inline_code(model));
    }
    let _ = writeln!(out, "- Session: {}", inline_code(&head.session_id));
    let _ = writeln!(
        out,
        "- Exported: {} from Vibe Work. Tool steps are summarised; reasoning and command output are left out.",
        utc_minute(head.exported_at_ms)
    );
    let mut body = String::new();
    let mut last_section: Option<&str> = None;
    let mut in_steps = false;
    for entry in entries {
        match (s(entry, "type"), s(entry, "role")) {
            (Some("message"), Some(role @ ("user" | "assistant"))) => {
                let text = message_body(entry);
                if text.is_empty() {
                    continue;
                }
                counts.messages += 1;
                let heading = if role == "user" { "You" } else { "Vibe" };
                // Consecutive answers of the agent stay in one section; steps between them close it.
                if last_section != Some(heading) || in_steps || role == "user" {
                    let when = entry
                        .get("createdAt")
                        .and_then(Value::as_u64)
                        .map(|ms| format!(" · {}", utc_minute(ms)))
                        .unwrap_or_default();
                    let _ = write!(body, "\n## {heading}{when}\n\n");
                } else {
                    body.push('\n');
                }
                let _ = writeln!(body, "{text}");
                last_section = Some(heading);
                in_steps = false;
            }
            (Some("effect"), _) => {
                if !in_steps {
                    body.push_str("\nSteps:\n\n");
                    in_steps = true;
                }
                counts.steps += 1;
                let _ = writeln!(body, "{}", step_line(entry));
                if let Some(diff) = effect_diff(entry) {
                    counts.diffs += 1;
                    let fence = fence_for(&diff);
                    let _ = write!(body, "\n{fence}diff\n{diff}\n{fence}\n\n");
                }
            }
            (Some("notice"), _) => {
                if let Some(text) = nonempty(s(entry, "message").or_else(|| s(entry, "text"))) {
                    let _ = write!(body, "\n> {}\n", text.replace('\n', "\n> "));
                    in_steps = false;
                }
            }
            _ => {}
        }
    }
    if body.trim().is_empty() {
        out.push_str("\n*This session has no messages yet.*\n");
    } else {
        out.push_str(&body);
    }
    let _ = writeln!(
        out,
        "\n---\n\n{} messages, {} steps, {} file changes.",
        counts.messages, counts.steps, counts.diffs
    );
    (out, counts)
}
