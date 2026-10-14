//! Which relative paths may be written into a new worktree. One rule for every operating system, because a
//! repository is shared across them and a name that is harmless on Linux can be a drive, a stream or a git
//! directory on Windows.
//!
//! Depends on: `SkipReason`. Called by: `plan`, `copy` (again, so a hand-built plan cannot bypass it).
//! Invariants: pure; returns the components or the reason; never touches the file system.

use super::SkipReason;

/// Windows device names (with any extension) that open a device instead of a file.
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Splits `rel` (a `/` separated repository-relative path) into names and checks every one.
///
/// Refused: an empty path, a leading or doubled `/`, `.` and `..`, a backslash or colon (a separator, a drive
/// or a stream on Windows), control characters (NUL included), a trailing dot or space (Windows drops them, so
/// `.git.` is `.git`), a Windows device name, and `.git` or its short form `git~1` in any case.
///
/// # Errors
/// [`SkipReason::UnsafeName`] with no detail: the caller lists the path as given.
pub fn validate_relative(rel: &str) -> Result<Vec<&str>, SkipReason> {
    if rel.is_empty() {
        return Err(SkipReason::UnsafeName);
    }
    let comps: Vec<&str> = rel.split('/').collect();
    for name in &comps {
        if !name_is_safe(name) {
            return Err(SkipReason::UnsafeName);
        }
    }
    Ok(comps)
}

fn name_is_safe(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    if name
        .chars()
        .any(|c| c.is_control() || c == '\\' || c == ':')
    {
        return false;
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    let lower = name.to_lowercase();
    if lower == ".git" || lower == "git~1" {
        return false;
    }
    let stem = lower.split('.').next().unwrap_or("");
    !RESERVED.contains(&stem.trim_end())
}
