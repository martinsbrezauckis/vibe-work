//! The bundled Handbook Format 1.2 stylesheet: bounded checks and an explicit installer for `handbook_check`.
//! Depends on the directory capability, export's regular-file reader and the embedded `STYLE`; called by the
//! validator and the core tool. Checks never write. Repair replaces only missing, unmarked or incomplete CSS, and
//! updates an earlier shipped 1.2 stylesheet (`EARLIER`) keeping rules appended after it; complete marked edits are
//! preserved. No write follows a link or modifies a hard-linked target in place.

use std::io::{ErrorKind, Write as _};
use std::path::Path;

use cap_std::fs::{Dir, OpenOptions};

use super::format::STYLESHEET;
use crate::STYLE;

const MARKER: &str = "/* Vibe Work handbook.css ";
const VERSION: &str = "1.2";
const FIX: &str = "handbook_check({repair: \"stylesheet\"})";
/// Earlier stylesheets Vibe Work shipped under the same "1.2" marker, oldest first. Changing
/// the rules without a new marker can make an earlier bundled stylesheet look like a user's edit (HB-E11).
/// Keep earlier bundled text here so upgrades do not ask the user to resolve it as an edit.
/// Add the previous text whenever `STYLE` changes without a new marker.
const EARLIER: &[&str] = &[include_str!("handbook-1.2-hb2.css")];
const TOKENS: &[&str] = &[
    "--bg", "--panel", "--raised", "--line", "--text", "--text-2", "--text-3", "--accent",
    "--blue", "--green", "--red",
];

/// A stylesheet error, with the explicit repair call; `None` for the bundled CSS plus optional appended rules.
#[must_use]
pub fn problem(root: &Path) -> Option<String> {
    match read(root) {
        Ok(Some(css)) => issue(&css).map(|reason| format!("{reason}; run {FIX}")),
        Ok(None) => Some(format!("handbook.css is missing; run {FIX}")),
        Err(reason) => Some(format!("{reason}; run {FIX} (unsafe files are refused)")),
    }
}

fn issue(css: &str) -> Option<&'static str> {
    if css.starts_with(STYLE) {
        None
    } else if earlier(css).is_some() {
        Some(
            "handbook.css is an earlier Vibe Work stylesheet; repair updates it and keeps rules appended after it",
        )
    } else if marker(css).is_some_and(|v| v != VERSION) {
        Some("handbook.css has an unknown version; marked file preserved for user review")
    } else if !complete(css) {
        Some("handbook.css is incomplete: missing :root variables for light or dark")
    } else if marker(css).is_none() {
        Some("handbook.css has no Vibe Work version marker")
    } else {
        Some(
            "handbook.css has a complete marked edit; preserved for user review (append custom rules to the bundled CSS)",
        )
    }
}

/// The earlier shipped stylesheet `css` starts with (line endings normalized), if any.
fn earlier(css: &str) -> Option<&'static str> {
    let css = css.replace("\r\n", "\n");
    EARLIER.iter().copied().find(|old| css.starts_with(old))
}

fn marker(css: &str) -> Option<&str> {
    css.trim_start()
        .strip_prefix(MARKER)?
        .split_once(" */")
        .map(|(v, _)| v)
}

/// Minimal structural check, not a CSS parser: token declarations must be nonempty inside the base and forced
/// dark root blocks, never merely mentioned by a `var()` reference, a body rule or a comment.
fn complete(css: &str) -> bool {
    let mut clean = String::new();
    let mut chars = css.chars().peekable();
    let mut quote = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            clean.push(c);
            if c == '\\' {
                if let Some(escaped) = chars.next() {
                    clean.push(escaped);
                }
            } else if c == q {
                quote = None;
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            loop {
                let Some(c) = chars.next() else { return false };
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            clean.push(' ');
        } else {
            if matches!(c, '\'' | '"') {
                quote = Some(c);
            }
            clean.push(c);
        }
    }
    let compact: String = clean
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .collect();
    let has = |selector: &str| {
        compact
            .split('{')
            .zip(compact.split('{').skip(1))
            .any(|(head, body)| {
                if !head
                    .rsplit('}')
                    .next()
                    .is_some_and(|h| h.split(',').any(|s| s == selector))
                {
                    return false;
                }
                let declarations = body.split('}').next().unwrap_or("");
                TOKENS.iter().all(|token| {
                    declarations.split(';').any(|d| {
                        d.split_once(':')
                            .is_some_and(|(name, value)| name == *token && !value.is_empty())
                    })
                })
            })
    };
    has(":root") && has(":root[data-theme=dark]")
}

/// Reads only a regular, unlinked CSS file through the handbook capability, up to the page input limit.
pub(crate) fn read(root: &Path) -> Result<Option<String>, String> {
    let dir = match Dir::open_ambient_dir(root, cap_std::ambient_authority()) {
        Ok(dir) => dir,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot open handbook folder: {e}")),
    };
    read_in(&dir)
}

fn read_in(dir: &Dir) -> Result<Option<String>, String> {
    read_named(dir, STYLESHEET)
}

fn read_named(dir: &Dir, name: &str) -> Result<Option<String>, String> {
    let meta = match dir.symlink_metadata(name) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot inspect handbook.css: {e}")),
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("handbook.css must be a regular file, not a link or special file".into());
    }
    let bytes = crate::export_files::read_beneath(dir, name, crate::html::MAX_INPUT)
        .map_err(|e| format!("cannot read handbook.css: {e}"))?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "handbook.css is not UTF-8".into())
}

/// Installs the shipped stylesheet in this project's relative handbook folder. Returns `created`, `restored`,
/// `unchanged` or `conflict` (the latter leaves complete marked edits and unknown versions untouched).
///
/// # Errors
/// An invalid folder, a link/special/oversized file, an I/O failure or an edit observed during repair.
pub fn repair(project: &Path, folder: &str) -> Result<&'static str, String> {
    if crate::clean_relative(folder).ok().as_deref() != Some(folder) {
        return Err("handbook folder must be a relative path inside the project".into());
    }
    let project =
        Dir::open_ambient_dir(project, cap_std::ambient_authority()).map_err(|e| e.to_string())?;
    project.create_dir_all(folder).map_err(|e| e.to_string())?;
    let dir = project.open_dir(folder).map_err(|e| e.to_string())?;
    let before = read_in(&dir)?;
    let mut content = STYLE.to_owned();
    if let Some(css) = &before {
        if issue(css).is_none() {
            return Ok("unchanged");
        }
        if let Some(old) = earlier(css) {
            // An earlier shipped stylesheet: the current one, then whatever the user appended after the old text.
            content.push_str(&css.replace("\r\n", "\n")[old.len()..]);
        } else if marker(css).is_some_and(|v| v != VERSION || complete(css)) {
            return Ok("conflict");
        }
    }
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let temp = format!(".handbook-css-{:016x}.tmp", u64::from_le_bytes(random));
    let result = (|| {
        let mut file = dir
            .open_with(&temp, OpenOptions::new().write(true).create_new(true))
            .map_err(|e| e.to_string())?;
        file.write_all(content.as_bytes())
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        if read_in(&dir)? != before {
            return Err("handbook.css changed during repair; run the check again".into());
        }
        install(&dir, &temp, before.as_deref())?;
        Ok(if before.is_some() {
            "restored"
        } else {
            "created"
        })
    })();
    let _ = dir.remove_file(&temp);
    result
}

/// Move the old file aside before inspecting it again, then install without replacing an existing name.
/// A competing pathname edit is either restored or retained under the backup name, never discarded.
fn install(dir: &Dir, temp: &str, before: Option<&str>) -> Result<(), String> {
    if before.is_none() {
        return dir
            .hard_link(temp, dir, STYLESHEET)
            .map_err(|e| e.to_string());
    }
    let backup = format!("{temp}.original");
    dir.rename(STYLESHEET, dir, &backup)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        if read_named(dir, &backup)?.as_deref() != before {
            return Err("handbook.css changed during repair; run the check again".to_owned());
        }
        // Unlike rename-over, hard_link fails if a writer created a new handbook.css after the move.
        dir.hard_link(temp, dir, STYLESHEET)
            .map_err(|e| e.to_string())
    })();
    if let Err(reason) = result {
        if dir.hard_link(&backup, dir, STYLESHEET).is_ok() {
            let _ = dir.remove_file(&backup);
            return Err(reason);
        }
        return Err(format!(
            "{reason}; original preserved at {backup}; review it before retrying"
        ));
    }
    dir.remove_file(&backup)
        .map_err(|e| format!("CSS installed; original retained at {backup}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_between_the_precheck_and_install_is_preserved() {
        let root = std::env::var_os("CARGO_TARGET_DIR")
            .map_or_else(
                || std::path::PathBuf::from("target"),
                std::path::PathBuf::from,
            )
            .join(format!("vw-hb2-race-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        dir.write("new.tmp", STYLE).unwrap();
        let edited = format!("{STYLE}\n/* user edit after precheck */\n");
        dir.write(STYLESHEET, &edited).unwrap();
        assert!(install(&dir, "new.tmp", Some("old incomplete CSS")).is_err());
        assert_eq!(dir.read_to_string(STYLESHEET).unwrap(), edited);
        assert!(!dir.try_exists("new.tmp.original").unwrap());
        // cap-std keeps directory handles open without FILE_SHARE_DELETE on Windows.
        drop(dir);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repair_releases_handles_before_an_edit_or_folder_removal() {
        let root = std::env::var_os("CARGO_TARGET_DIR")
            .map_or_else(
                || std::path::PathBuf::from("target"),
                std::path::PathBuf::from,
            )
            .join(format!("vw-ci4f-repair-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let folder = "docs/handbook";
        let css = root.join(folder).join(STYLESHEET);
        assert_eq!(repair(&root, folder).unwrap(), "created");
        // The next edit must be possible after both a read-only check and an install.
        assert!(problem(&root.join(folder)).is_none());
        std::fs::write(&css, "old incomplete CSS").unwrap();
        assert_eq!(repair(&root, folder).unwrap(), "restored");
        assert_eq!(std::fs::read_to_string(&css).unwrap(), STYLE);
        let edited = format!("{STYLE}\n/* user edit */\n");
        std::fs::write(&css, &edited).unwrap();
        assert_eq!(repair(&root, folder).unwrap(), "unchanged");
        assert_eq!(std::fs::read_to_string(&css).unwrap(), edited);
        std::fs::remove_dir_all(root).unwrap();
    }
}
