//! Tracks project folders seen in Vibe sessions for the Projects overview.
//!
//! Depends on: `core::Shared`, `session_listing`, and `pages_overview`.
//! The set grows during a run from Vibe's own session data and explicitly opened projects.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;
use vw_platform::sync::LockRecover;

use crate::core::Shared;

/// Project folders known to the Projects overview.
pub(crate) struct PlanProfile {
    projects: Mutex<BTreeSet<String>>,
}

impl PlanProfile {
    /// Starts with no known projects.
    pub(crate) fn new() -> Self {
        Self {
            projects: Mutex::new(BTreeSet::new()),
        }
    }

    fn add(&self, folders: impl IntoIterator<Item = String>) {
        let mut projects = self.projects.lock_or_recover();
        for folder in folders.into_iter().filter(|f| !f.trim().is_empty()) {
            if let Ok(resolved) = std::fs::canonicalize(&folder) {
                let resolved = resolved.to_string_lossy().into_owned();
                // `canonicalize` on Windows answers `\\?\C:\...`; Vibe reports `C:\...`.
                let resolved = resolved
                    .strip_prefix(r"\\?\")
                    .map(str::to_owned)
                    .unwrap_or(resolved);
                projects.insert(resolved);
            }
            projects.insert(folder);
        }
    }
}

/// Removes the retired app-owned agent file from a prior Vibe Work install. Content that does not
/// match its exact old scaffold, symlinks, and anything outside this literal path are kept.
pub(crate) fn retire_legacy_agent(vibe_home: &Path) -> std::io::Result<bool> {
    let agents = vibe_home.join("agents");
    let parent = match std::fs::symlink_metadata(&agents) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !parent.file_type().is_dir() {
        return Ok(false);
    }
    let file = agents.join("vw-plan.toml");
    let metadata = match std::fs::symlink_metadata(&file) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() || metadata.len() > 1_048_576 {
        return Ok(false);
    }
    let body = std::fs::read_to_string(&file)?;
    if !was_generated_plan_agent(&body) {
        return Ok(false);
    }
    // Recheck the file type at the deletion point, including a symlink substituted since read.
    if !std::fs::symlink_metadata(&file)?.file_type().is_file() {
        return Ok(false);
    }
    std::fs::remove_file(file)?;
    Ok(true)
}

/// Recognizes the exact scaffold written by the removed 0.1.15 generator. The only variable part
/// is the sorted plan-path allowlist, which must appear identically under `write_file` and edit.
fn was_generated_plan_agent(body: &str) -> bool {
    let mut lines = body.lines();
    let fixed = [
        "# Written by Vibe Work: the \"Plan on a page\" access mode. Rewritten when projects change; edits here are",
        "# replaced. Plan pages may be written; every other file write or edit is refused. Shell commands still run.",
        "display_name = \"Plan on a page\"",
        "description = \"Plans first and writes the plan as a page in the project's handbook (docs/handbook/plans/). Other file writes are refused; shell commands still run.\"",
        "safety = \"safe\"",
        "disabled_tools = []",
        "",
        "[tools.write_file]",
        "permission = \"never\"",
        "allowlist = [",
    ];
    if !fixed.iter().all(|expected| lines.next() == Some(*expected)) {
        return false;
    }
    let mut write_list = Vec::new();
    loop {
        let Some(line) = lines.next() else {
            return false;
        };
        if line == "]" {
            break;
        }
        if line.is_empty() {
            if lines.next() != Some("]") || !write_list.is_empty() {
                return false;
            }
            write_list.push(line);
            break;
        }
        if !line.starts_with("  \"") || !line.ends_with("\",") {
            return false;
        }
        write_list.push(line);
    }
    if !valid_legacy_list(&write_list) {
        return false;
    }
    if ![
        "",
        "[tools.edit]",
        "permission = \"never\"",
        "allowlist = [",
    ]
    .iter()
    .all(|expected| lines.next() == Some(*expected))
    {
        return false;
    }
    for line in write_list {
        if lines.next() != Some(line) {
            return false;
        }
    }
    lines.next() == Some("]") && lines.next().is_none()
}

fn valid_legacy_list(lines: &[&str]) -> bool {
    let list = format!("paths = [\n{}\n]", lines.join("\n"));
    let Ok(table) = list.parse::<toml::Table>() else {
        return false;
    };
    let Some(paths) = table.get("paths").and_then(toml::Value::as_array) else {
        return false;
    };
    let mut previous: Option<&str> = None;
    for path in paths {
        let Some(path) = path.as_str() else {
            return false;
        };
        let Some(prefix) = path
            .strip_suffix("/plans/*")
            .or_else(|| path.strip_suffix("\\plans\\*"))
        else {
            return false;
        };
        let bytes = prefix.as_bytes();
        let absolute = prefix.starts_with('/')
            || prefix.starts_with("\\\\")
            || (bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/'));
        if !absolute || prefix.contains(['*', '?', '[', ']']) {
            return false;
        }
        if previous.is_some_and(|before| before >= path) {
            return false;
        }
        previous = Some(path);
    }
    true
}

/// Notes a project folder opened in the Projects overview.
pub(crate) fn note_project(shared: &Shared, folder: &str) {
    shared.plan_profile.add([folder.to_owned()]);
}

/// Notes project folders from a `session/list` page.
pub(crate) fn note_listing(shared: &Shared, items: &[Value]) {
    let folders = items
        .iter()
        .filter_map(|item| item.get("cwd").and_then(Value::as_str))
        .map(str::to_owned);
    shared.plan_profile.add(folders);
}

/// Every project folder known in this run (the workspace hub).
pub(crate) fn projects(shared: &Shared) -> Vec<String> {
    let set = shared.plan_profile.projects.lock_or_recover();
    // A resolved duplicate of a symlinked folder is shown once (by the folder Vibe reports).
    let mut seen = BTreeSet::new();
    set.iter()
        .filter(|p| {
            let key = std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p.as_str()));
            seen.insert(key)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod retired_agent_tests {
    use super::retire_legacy_agent;
    use std::fs;

    const GENERATED: &str = "# Written by Vibe Work: the \"Plan on a page\" access mode. Rewritten when projects change; edits here are\n\
# replaced. Plan pages may be written; every other file write or edit is refused. Shell commands still run.\n\
display_name = \"Plan on a page\"\n\
description = \"Plans first and writes the plan as a page in the project's handbook (docs/handbook/plans/). Other file writes are refused; shell commands still run.\"\n\
safety = \"safe\"\n\
disabled_tools = []\n\
\n\
[tools.write_file]\n\
permission = \"never\"\n\
allowlist = [\n\
\x20\x20\"/project/docs/handbook/plans/*\",\n\
]\n\
\n\
[tools.edit]\n\
permission = \"never\"\n\
allowlist = [\n\
\x20\x20\"/project/docs/handbook/plans/*\",\n\
]\n";

    #[test]
    fn startup_removes_only_the_old_app_generated_plan_agent() {
        let home = tempfile::tempdir().unwrap();
        assert!(!retire_legacy_agent(home.path()).unwrap());
        let agents = home.path().join("agents");
        fs::create_dir(&agents).unwrap();
        assert!(!retire_legacy_agent(home.path()).unwrap());
        let file = agents.join("vw-plan.toml");
        fs::write(&file, GENERATED).unwrap();
        assert!(retire_legacy_agent(home.path()).unwrap());
        assert!(!file.exists());

        let empty = GENERATED.replace("  \"/project/docs/handbook/plans/*\",\n", "\n");
        fs::write(&file, empty).unwrap();
        assert!(retire_legacy_agent(home.path()).unwrap());
        assert!(!file.exists());

        fs::write(&file, "").unwrap();
        assert!(!retire_legacy_agent(home.path()).unwrap());
        assert!(file.exists());

        fs::write(&file, format!("{GENERATED}\n# user's custom edit\n")).unwrap();
        assert!(!retire_legacy_agent(home.path()).unwrap());
        assert!(file.exists());
        fs::write(&file, "display_name = \"My agent\"\n").unwrap();
        assert!(!retire_legacy_agent(home.path()).unwrap());
        assert!(file.exists());
        fs::write(
            &file,
            GENERATED.replace("/project/docs/handbook/plans/*", "/project/unrelated/*"),
        )
        .unwrap();
        assert!(!retire_legacy_agent(home.path()).unwrap());
        assert!(file.exists());
    }

    #[cfg(unix)]
    #[test]
    fn startup_preserves_plan_agent_symlink() {
        let home = tempfile::tempdir().unwrap();
        let agents = home.path().join("agents");
        fs::create_dir(&agents).unwrap();
        let target = home.path().join("custom.toml");
        fs::write(&target, GENERATED).unwrap();
        let link = agents.join("vw-plan.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(!retire_legacy_agent(home.path()).unwrap());
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), GENERATED);
    }

    #[cfg(unix)]
    #[test]
    fn startup_preserves_symlinked_agents_directory() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("vw-plan.toml");
        fs::write(&file, GENERATED).unwrap();
        std::os::unix::fs::symlink(outside.path(), home.path().join("agents")).unwrap();
        assert!(!retire_legacy_agent(home.path()).unwrap());
        assert_eq!(fs::read_to_string(&file).unwrap(), GENERATED);
    }
}
