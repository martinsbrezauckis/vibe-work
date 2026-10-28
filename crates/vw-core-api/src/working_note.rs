//! The always-on "where you are" note: a short map that tells the agent it runs
//! inside Vibe Work (not the Vibe command line), which Vibe Work skills exist and when to load each one.
//!
//! Delivery: Vibe puts `$VIBE_HOME/AGENTS.md` into every session's system prompt as the user-level instructions
//! (`vibe/core/config/harness_files/_harness_manager.py` `load_user_doc`, rendered by `core/system_prompt.py`
//! `get_agents_md_section`; Vibe 2.26.1). Vibe Work owns its `VIBE_HOME`, so the note lives there as a
//! marked block at the top of that file, the same pattern as our `hooks.toml` block. Other channels were checked and
//! not used: the session's `agentConfig.instructions` is accepted but has no reader in the 2.26.1 source; `project_instructions`
//! exist only for in-process hosts; an agent profile's `instructions` would mean replacing Vibe's built-in access
//! modes; skill and plugin descriptions are routing text the model may skip (no skill loaded on a feature task).
//!
//! The same file is the user's personal instructions (Settings): [`strip`] hides the block from the
//! editor and [`keep`] puts it back in front of what the user saves, so the user only ever sees and edits their own
//! text. Only the exact block this build ships, at the very start of the file, is ever hidden ([`is_ours`]); any other
//! marker text stays visible in the editor. The start-up [`write`] replaces a leading block of ours from an earlier
//! version (first line `BEGIN` up to the first `END` line).
//!
//! Depends on: std. Called by: `instructions` (personal read and write), `vw-host::agent_tools::prepare` (writes
//! or removes the block at every start).
//! Invariants: the block is the first thing in the file and appears at most once; text outside it is kept byte for
//! byte; a link at the file's place is never written through.

use std::fs;
use std::io::Write;
use std::path::Path;

/// First line of the block.
pub const BEGIN: &str = "<!-- >>> vibe-work note (managed by Vibe Work: rewritten at every start; your own instructions go below the end line) >>> -->";
/// Last line of the block.
pub const END: &str = "<!-- <<< vibe-work note <<< -->";
/// The note itself (`working_note.md`): the map, a few hundred tokens.
pub const TEXT: &str = include_str!("working_note.md");
const SCREEN_READER: &str = if cfg!(target_os = "macos") {
    "On macOS, the system screen reader is VoiceOver (System Settings > Accessibility)."
} else if cfg!(windows) {
    "On Windows, the system screen reader is Narrator (Settings > Accessibility)."
} else {
    "On Linux, the desktop screen reader is Orca (Accessibility settings)."
};
/// The same explanation is delivered to the model and the window.
pub const MCP_DISABLED: &str = "Vibe Work's tools are unavailable because enable_mcp = false in its managed config.toml. Enable MCP there and reopen the chat to use them.";

/// Reads only the managed configuration's explicit MCP switch.
#[must_use]
pub fn mcp_disabled(vibe_home: &Path) -> bool {
    fs::read_to_string(vibe_home.join("config.toml"))
        .ok()
        .and_then(|text| toml::from_str::<toml::Value>(&text).ok())
        .and_then(|config| config.get("enable_mcp").and_then(toml::Value::as_bool))
        == Some(false)
}

fn disabled_block() -> String {
    format!(
        "{BEGIN}\n{MCP_DISABLED}\n\n{}\n\n{SCREEN_READER}\n{END}\n",
        TEXT.trim_end()
    )
}

fn recognised(doc: &str) -> Option<String> {
    [block(), disabled_block()]
        .into_iter()
        .find(|prefix| doc.starts_with(prefix))
}

/// The whole block: markers around [`TEXT`], ending in a newline.
#[must_use]
pub fn block() -> String {
    format!("{BEGIN}\n{}\n\n{SCREEN_READER}\n{END}\n", TEXT.trim_end())
}

/// True when `doc` starts with exactly the block this build ships (its first bytes are [`block`]). Only then is the
/// block ours to hide or keep; anything else stays visible text.
#[must_use]
pub fn is_ours(doc: &str) -> bool {
    recognised(doc).is_some()
}

/// `doc` without our block: the user's own text. Only the exact block this build ships, at the very start of the
/// file, is removed (with the one blank line we put after it). A marker elsewhere, a block with other text inside or
/// a block without its end line is not ours to hide: the document comes back unchanged, so the user sees everything
/// that reaches the model (hiding any text between markers would let a planted block reach the
/// model unseen).
#[must_use]
pub fn strip(doc: &str) -> String {
    let Some(prefix) = recognised(doc) else {
        return doc.to_owned();
    };
    let rest = &doc[prefix.len()..];
    rest.strip_prefix('\n').unwrap_or(rest).to_owned()
}

/// The file text for the user's `text`: with our block in front when `note` is true, else `text` alone (a leading
/// copy of the exact block in `text` is not doubled).
#[must_use]
pub fn compose(text: &str, note: bool) -> String {
    let user = strip(text);
    if !note {
        return user;
    }
    if user.is_empty() {
        block()
    } else {
        format!("{}\n{user}", block())
    }
}

/// What to write when the user saves `text` over a file that held `existing`: our block stays when the file started
/// with it.
#[must_use]
pub fn keep(existing: &str, text: &str) -> String {
    let composed = compose(text, is_ours(existing));
    if existing.starts_with(&disabled_block()) {
        composed.replacen(&block(), &disabled_block(), 1)
    } else {
        composed
    }
}

/// True when `raw` (the file on disk) is what the user was shown as `expected` (the stale-write check of the
/// editor): `raw` is `expected` itself, or our exact block followed by `expected`.
#[must_use]
pub fn shows_as(raw: &str, expected: &str) -> bool {
    if is_ours(raw) {
        keep(raw, expected) == raw && strip(raw) == expected
    } else {
        raw == expected
    }
}

/// The file's text without a block of ours that the app wrote at an earlier start, possibly with an older note text:
/// only at the very start (first line exactly [`BEGIN`]) and only up to the first line exactly [`END`]. Used by the
/// start-up writer, never by the editor: text between our markers at the top is declared "managed by Vibe Work:
/// rewritten at every start" and is replaced, not hidden. Without an end line nothing is removed.
fn without_leading_block(doc: &str) -> &str {
    let Some(rest) = doc.strip_prefix(BEGIN) else {
        return doc;
    };
    let Some(rest) = rest.strip_prefix('\n') else {
        return doc;
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end_matches(['\r', '\n']) == END {
            let after = &rest[offset..];
            return after.strip_prefix('\n').unwrap_or(after);
        }
    }
    doc
}

/// Writes (`note = true`) or removes (`false`) the block in `vibe_home/AGENTS.md`, keeping the user's text. Returns
/// whether the file changed.
///
/// # Errors
/// A text reason when the file is a link or not plain text (left alone), or cannot be read or written.
pub fn write(vibe_home: &Path, note: bool) -> Result<bool, String> {
    let path = vibe_home.join("AGENTS.md");
    let existing = match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            return Err(format!(
                "{} is a link or a folder: the Vibe Work note was not written",
                path.display()
            ));
        }
        Ok(_) => fs::read(&path)
            .map_err(|e| format!("{}: {e}", path.display()))
            .and_then(|bytes| {
                String::from_utf8(bytes).map_err(|_| {
                    format!(
                        "{} is not UTF-8 text: the note was not written",
                        path.display()
                    )
                })
            })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut updated = compose(without_leading_block(&existing), note);
    if note && mcp_disabled(vibe_home) {
        updated = updated.replacen(&block(), &disabled_block(), 1);
    }
    if updated == existing {
        return Ok(false);
    }
    if updated.is_empty() {
        // Only our block was there: remove the file rather than leave an empty one.
        fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok(true);
    }
    fs::create_dir_all(vibe_home).map_err(|e| format!("{}: {e}", vibe_home.display()))?;
    let tmp = vibe_home.join(format!(".AGENTS.md.note.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(updated.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(true)
}
