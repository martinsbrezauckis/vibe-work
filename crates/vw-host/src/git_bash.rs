//! Windows: Vibe's Git Bash runs inside a kill-on-close job, so Stop and the bash timeout end every program a command
//! started (`vw_agent_tools::git_bash`).
//!
//! At each start, when the session tools are on and Vibe can find Git Bash:
//! - the tools binary is copied to `<data>/shell/bash.exe` (Vibe accepts a configured Git Bash only by that name; the
//!   copy is refreshed when it differs, written beside and renamed over);
//! - Vibe Work's own `VIBE_HOME/config.toml` gets `[tools.git_bash] shell = "<that copy>"`;
//! - the Vibe server's environment gets [`vw_agent_tools::git_bash::ENV_REAL`] = the real Git Bash.
//!
//! A `shell` the user set to something else is kept (that shell then runs without the job, as before). Without Git
//! Bash, without the tools binary or when the copy fails, our `shell` line is removed again: Vibe then finds its own
//! shell as before, and a stale line never points Vibe at a wrapper with no real Git Bash behind it.
//!
//! Depends on: `toml_edit` (the file keeps its comments and layout), `vw_platform::replace_private`. Called by:
//! `agent_tools::prepare` (Windows), tests (the pure parts on every OS).
//! Invariants: only Vibe Work's own `VIBE_HOME`; a file that is not valid TOML is left byte-identical.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, value};

/// The copy's folder under the data folder.
pub const DIR: &str = "shell";

/// What the config should say about our wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted<'a> {
    /// `[tools.git_bash] shell` is our wrapper at this path.
    Ours(&'a str),
    /// No wrapper this run: our line (the wrapper at this path) goes, anything else stays.
    None(&'a str),
}

/// What [`with_shell`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shell {
    /// The text changed; the new text.
    Changed(String),
    /// Already as wanted.
    Same,
    /// The user set another shell: kept.
    UsersOwn(String),
    /// Not valid TOML, or `tools` / `tools.git_bash` is not a table: left as it is.
    Skipped,
}

/// The config text with `[tools.git_bash] shell` as `wanted` asks. Pure.
#[must_use]
pub fn with_shell(existing: &str, wanted: &Wanted<'_>) -> Shell {
    let Ok(mut config) = existing.parse::<DocumentMut>() else {
        return Shell::Skipped;
    };
    let ours = match wanted {
        Wanted::Ours(path) | Wanted::None(path) => *path,
    };
    let current = config
        .get("tools")
        .and_then(|tools| tools.get("git_bash"))
        .and_then(|bash| bash.get("shell"))
        .map(|shell| shell.as_str().map(str::to_owned));
    match (wanted, current) {
        (_, Some(Some(shell))) if shell != ours => Shell::UsersOwn(shell),
        (_, Some(None)) => Shell::Skipped,
        (Wanted::Ours(_), Some(Some(_))) | (Wanted::None(_), None) => Shell::Same,
        (Wanted::None(_), Some(Some(_))) => {
            let Some(bash) = config
                .get_mut("tools")
                .and_then(|tools| tools.get_mut("git_bash"))
                .and_then(Item::as_table_like_mut)
            else {
                return Shell::Skipped;
            };
            bash.remove("shell");
            Shell::Changed(config.to_string())
        }
        (Wanted::Ours(path), None) => {
            let tools = config
                .entry("tools")
                .or_insert_with(|| {
                    let mut table = Table::new();
                    table.set_implicit(true);
                    Item::Table(table)
                })
                .as_table_like_mut();
            let Some(tools) = tools else {
                return Shell::Skipped;
            };
            let Some(bash) = tools
                .entry("git_bash")
                .or_insert(Item::Table(Table::new()))
                .as_table_like_mut()
            else {
                return Shell::Skipped;
            };
            bash.insert("shell", value(*path));
            Shell::Changed(config.to_string())
        }
    }
}

/// Git Bash as Vibe 3.0.2 finds it (`vibe/_processes/_windows.py` `_find_git_bash`): `bash.exe` on `PATH` unless it is
/// WSL's (`System32`) or the Store alias (`WindowsApps`); else next to `git.exe` on `PATH` (`..\bin\bash.exe`,
/// `..\usr\bin\bash.exe`); else under `ProgramFiles`, `ProgramFiles(x86)`, `LOCALAPPDATA` (`Git\bin\bash.exe`,
/// `Programs\Git\bin\bash.exe`). `var` reads an environment variable, `is_file` checks a path. Pure.
#[must_use]
pub fn find_real(
    var: impl Fn(&str) -> Option<String>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let path = var("PATH").unwrap_or_default();
    let on_path = |name: &str| {
        path.split(';')
            .filter(|dir| !dir.is_empty())
            .map(|dir| Path::new(dir.trim_matches('"')).join(name))
            .find(|candidate| is_file(candidate))
    };
    if let Some(bash) = on_path("bash.exe") {
        let normalized = bash.to_string_lossy().replace('\\', "/").to_lowercase();
        if !normalized.contains("/system32/bash.exe")
            && !normalized.contains("/windowsapps/bash.exe")
        {
            return Some(bash);
        }
    }
    if let Some(root) =
        on_path("git.exe").and_then(|git| git.parent()?.parent().map(Path::to_path_buf))
    {
        for relative in ["bin/bash.exe", "usr/bin/bash.exe"] {
            let candidate = root.join(relative);
            if is_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        let Some(root) = var(variable) else { continue };
        for relative in ["Git/bin/bash.exe", "Programs/Git/bin/bash.exe"] {
            let candidate = Path::new(&root).join(relative);
            if is_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Copies `tools` to `<data>/shell/bash.exe` unless the copy there is already the same file; returns the copy.
///
/// # Errors
/// The folder, the copy or the rename failed (an old copy in use by a still-running command keeps the rename from
/// replacing it).
pub fn place_copy(tools: &Path, data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = data_dir.join(DIR);
    std::fs::create_dir_all(&dir)?;
    let copy = dir.join(vw_agent_tools::git_bash::FILE_NAME);
    if std::fs::read(&copy).ok() == Some(std::fs::read(tools)?) {
        return Ok(copy);
    }
    let fresh = copy.with_extension("new");
    std::fs::copy(tools, &fresh)?;
    std::fs::rename(&fresh, &copy)?;
    Ok(copy)
}

/// Brings Vibe Work's `vibe_home/config.toml` in line with `wanted`; a line for the log.
#[must_use]
pub fn write_config(vibe_home: &Path, wanted: &Wanted<'_>) -> String {
    let path = vibe_home.join("config.toml");
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return format!("config.toml not read: {error}"),
    };
    match with_shell(&existing, wanted) {
        Shell::Changed(updated) => match vw_platform::replace_private(&path, updated.as_bytes()) {
            Ok(()) => "config updated".to_owned(),
            Err(error) => format!("config.toml not written: {error}"),
        },
        Shell::Same => "config already set".to_owned(),
        Shell::UsersOwn(shell) => {
            format!(
                "the config names its own Git Bash ({shell}), kept: Stop may leave its programs running"
            )
        }
        Shell::Skipped => {
            "config.toml not changed (not valid TOML or an unexpected tools table)".to_owned()
        }
    }
}

/// The Windows start-up step (see the module text); a line for the log.
#[cfg(windows)]
#[must_use]
pub fn prepare(
    supervisor: &mut vw_supervisor::SupervisorConfig,
    data_dir: &Path,
    tools: Option<&Path>,
) -> String {
    let ours = data_dir.join(DIR).join(vw_agent_tools::git_bash::FILE_NAME);
    let ours_text = ours.to_string_lossy().into_owned();
    let real = find_real(|name| std::env::var(name).ok(), Path::is_file);
    let outcome = match (tools, real) {
        (Some(tools), Some(real)) => place_copy(tools, data_dir)
            .map(|copy| (copy, real))
            .map_err(|error| error.to_string()),
        (None, _) => Err("the tools binary is missing".to_owned()),
        (_, None) => Err("Vibe finds no Git Bash".to_owned()),
    };
    match outcome {
        Ok((copy, real)) => {
            let wanted = copy.to_string_lossy().into_owned();
            let config = write_config(&supervisor.vibe_home, &Wanted::Ours(&wanted));
            supervisor.extra_env.push((
                vw_agent_tools::git_bash::ENV_REAL.to_owned(),
                real.clone().into_os_string(),
            ));
            format!(
                "Git Bash in a job: {} runs {} ({config})",
                copy.display(),
                real.display()
            )
        }
        Err(why) => {
            let config = write_config(&supervisor.vibe_home, &Wanted::None(&ours_text));
            format!("Git Bash without a job: {why} ({config})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &str = r"C:\Users\u\AppData\Roaming\app.vibework.desktop\shell\bash.exe";

    #[test]
    fn our_shell_is_added_kept_and_removed_but_the_users_own_stays() {
        let Shell::Changed(added) = with_shell("active_model = \"large-4\"\n", &Wanted::Ours(OURS))
        else {
            panic!("not added");
        };
        let parsed: DocumentMut = added.parse().unwrap_or_default();
        assert_eq!(parsed["tools"]["git_bash"]["shell"].as_str(), Some(OURS));
        assert_eq!(parsed["active_model"].as_str(), Some("large-4"));
        assert_eq!(with_shell(&added, &Wanted::Ours(OURS)), Shell::Same);
        let Shell::Changed(removed) = with_shell(&added, &Wanted::None(OURS)) else {
            panic!("not removed");
        };
        assert!(!removed.contains("bash.exe"), "{removed}");
        assert_eq!(with_shell("", &Wanted::None(OURS)), Shell::Same);

        let users = "[tools.git_bash]\npermission = \"ask\"\nshell = 'D:\\Git\\bin\\bash.exe'\n";
        assert_eq!(
            with_shell(users, &Wanted::Ours(OURS)),
            Shell::UsersOwn(r"D:\Git\bin\bash.exe".to_owned())
        );
        assert!(matches!(
            with_shell(users, &Wanted::None(OURS)),
            Shell::UsersOwn(_)
        ));
        // A table of the user's keeps its other keys; comments stay.
        let Shell::Changed(merged) = with_shell(
            "# mine\n[tools.git_bash]\npermission = \"ask\"\n",
            &Wanted::Ours(OURS),
        ) else {
            panic!("not merged");
        };
        assert!(
            merged.contains("# mine") && merged.contains("permission = \"ask\""),
            "{merged}"
        );
        assert_eq!(
            with_shell("not = [toml", &Wanted::Ours(OURS)),
            Shell::Skipped
        );
    }

    #[test]
    fn git_bash_is_found_where_vibe_finds_it() {
        // Forward slashes: the same paths on every OS (Windows reads them too).
        let files = [
            "C:/Windows/System32/bash.exe",
            "C:/Program Files/Git/cmd/git.exe",
            "C:/Program Files/Git/bin/bash.exe",
        ];
        let is_file = |path: &Path| files.iter().any(|f| Path::new(f) == path);
        let env = |path: &'static str| move |name: &str| (name == "PATH").then(|| path.to_owned());
        // WSL's bash on PATH is skipped; the Git next to git.exe is found.
        assert_eq!(
            find_real(env("C:/Windows/System32;C:/Program Files/Git/cmd"), is_file),
            Some(PathBuf::from("C:/Program Files/Git/bin/bash.exe"))
        );
        assert_eq!(find_real(env("C:/Windows/System32"), is_file), None);
        let program_files =
            |name: &str| (name == "ProgramFiles").then(|| "C:/Program Files".to_owned());
        assert_eq!(
            find_real(program_files, is_file),
            Some(PathBuf::from("C:/Program Files/Git/bin/bash.exe"))
        );
    }
}
