//! The app-owned `vw-plan` agent profile: Vibe's plan rules plus write permission for the
//! plan pages of every known project, written to `VIBE_HOME/agents/vw-plan.toml`.
//!
//! Facts (Vibe 2.26.0): custom profiles are read when a session is opened (`core/agents/registry.py`, built by
//! `AgentManager` in `app_server/_runtime.py:1485`), so the file must name a project before its session opens;
//! allowlist entries are matched with `PurePath.match` when absolute (`permissions.py:91-111`): `*` stays inside
//! one folder, relative entries never match an absolute path.
//!
//! Depends on: shared vocabulary. Called by: vw-core-api (`plan_profile`), tests.
//! Invariants:
//! - Only absolute entries: `VIBE_HOME/plans/*` and `<project>/<handbook folder>/plans/*` (Windows projects with `\`).
//! - A project whose path or pages folder contains a glob character (`*`, `?`, `[`, `]`) is left out (its entry
//!   could match other folders); so is a relative project path.
//! - The text is valid TOML (every string escaped); `bash` is not disabled, so the profile is limited, never
//!   read-only (it is described that way).

use std::path::Path;

/// The profile's name (the file stem) as `session/agent/update` takes it.
pub const NAME: &str = "vw-plan";

/// One project the profile covers.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlanTarget {
    /// The project folder as Vibe reports it (the session's `cwd`).
    pub project: String,
    /// The pages folder relative to it (`docs/pages`).
    pub pages_folder: String,
}

fn is_windows_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    (bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/'))
        || path.starts_with("\\\\")
}

fn has_glob(text: &str) -> bool {
    text.contains(['*', '?', '[', ']'])
}

/// The allowlist entry for one project, or `None` when it must be left out (see the module invariants).
#[must_use]
pub fn pattern(target: &PlanTarget) -> Option<String> {
    let project = target.project.trim_end_matches(['/', '\\']);
    if project.is_empty() || has_glob(project) || has_glob(&target.pages_folder) {
        return None;
    }
    let windows = is_windows_path(project);
    if !windows && !project.starts_with('/') {
        return None;
    }
    let sep = if windows { "\\" } else { "/" };
    let folder = target
        .pages_folder
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(sep);
    Some(format!("{project}{sep}{folder}{sep}plans{sep}*"))
}

/// A TOML basic string.
fn toml_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The profile text for `targets` (sorted, deduplicated) and Vibe's own plans folder `vibe_home/plans`.
#[must_use]
pub fn toml(vibe_home: &Path, targets: &[PlanTarget]) -> String {
    let home = vibe_home.to_string_lossy();
    let home = home.trim_end_matches(['/', '\\']);
    let sep = if is_windows_path(home) { "\\" } else { "/" };
    let mut entries = vec![format!("{home}{sep}plans{sep}*")];
    let mut patterns: Vec<String> = targets.iter().filter_map(pattern).collect();
    patterns.sort();
    patterns.dedup();
    entries.extend(patterns);
    let list = entries
        .iter()
        .map(|e| format!("  {},", toml_string(e)))
        .collect::<Vec<_>>()
        .join("\n");
    let plan_name = toml_string(vw_vocabulary::label("accessModes", NAME));
    let plan_description =
        toml_string(vw_vocabulary::term("accessModes", NAME).map_or("", |term| term.description));
    format!(
        "# Written by Vibe Work: the {plan_name} access mode. Rewritten when projects change; edits here are\n\
         # replaced. Plan pages may be written; every other file write or edit is refused. Shell commands still run.\n\
         display_name = {plan_name}\n\
         description = {plan_description}\n\
         safety = \"safe\"\n\
         disabled_tools = []\n\
         \n\
         [tools.write_file]\n\
         permission = \"never\"\n\
         allowlist = [\n{list}\n]\n\
         \n\
         [tools.edit]\n\
         permission = \"never\"\n\
         allowlist = [\n{list}\n]\n"
    )
}
