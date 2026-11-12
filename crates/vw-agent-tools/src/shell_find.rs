//! Which real shell the `bash.exe` and PowerShell modes run (Windows), and how Vibe itself finds Git Bash and
//! PowerShell. Vibe Work's `vw-host` uses the same discovery at start-up to decide what to name in Vibe Work's
//! `VIBE_HOME/config.toml`; the modes use it when the server's environment does not name the real shell.
//!
//! Why the modes need it: the `[tools.git_bash] shell` (or `[tools.powershell] shell`) line names our copy, and a Vibe
//! CLI the person starts in a terminal with the same `VIBE_HOME` reads that line too, but its environment has no
//! `VW_GIT_BASH` / `VW_POWERSHELL` (only the app's Vibe server gets them). Without a fallback every command and every
//! hook (the strict kill-guard pre-tool hook runs through this shell) failed with 127, so the CLI denied every tool
//! call. With it, the copy runs the Git Bash (or PowerShell) Vibe would have found itself.
//!
//! Depends on: nothing. Called by: `git_bash::run_real` (Windows), `vw-host` (`git_bash`, `powershell_shell`), tests
//! (pure; Windows path rules on every OS).

use std::path::{Path, PathBuf};

/// The shells the modes stand in for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Git Bash (`bash.exe` mode).
    GitBash,
    /// PowerShell 7 or Windows PowerShell (PowerShell mode).
    PowerShell,
}

impl Kind {
    /// The program Vibe itself would find for this shell. Pure.
    #[must_use]
    pub fn find(
        self,
        var: impl Fn(&str) -> Option<String>,
        is_file: impl Fn(&Path) -> bool,
    ) -> Option<PathBuf> {
        match self {
            Self::GitBash => git_bash(var, is_file),
            Self::PowerShell => powershell(var, is_file),
        }
    }
}

/// The real program a mode runs: `env_real`'s value when it is an absolute path to an existing file, else the one
/// Vibe would find itself ([`Kind::find`]; a Vibe CLI started outside Vibe Work). `is_file` must be false for this
/// program itself (a copy of it found on `PATH` would start itself without end). Pure.
#[must_use]
pub fn resolve_real(
    kind: Kind,
    env_real: &str,
    var: impl Fn(&str) -> Option<String>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    var(env_real)
        .map(PathBuf::from)
        .filter(|named| named.is_absolute() && is_file(named))
        .or_else(|| kind.find(&var, &is_file))
}

/// The directories of a Windows `PATH` value joined with `name`, in order (quotes around an entry removed).
fn on_path<'a>(path: &'a str, name: &'a str) -> impl Iterator<Item = PathBuf> + 'a {
    path.split(';')
        .filter(|dir| !dir.is_empty())
        .map(move |dir| Path::new(dir.trim_matches('"')).join(name))
}

/// Git Bash as Vibe 3.0.2 finds it (`vibe/_processes/_windows.py` `_find_git_bash`): `bash.exe` on `PATH` unless it is
/// WSL's (`System32`) or the Store alias (`WindowsApps`); else next to `git.exe` on `PATH` (`..\bin\bash.exe`,
/// `..\usr\bin\bash.exe`); else under `ProgramFiles`, `ProgramFiles(x86)`, `LOCALAPPDATA` (`Git\bin\bash.exe`,
/// `Programs\Git\bin\bash.exe`). `var` reads an environment variable, `is_file` checks a path. Pure.
#[must_use]
pub fn git_bash(
    var: impl Fn(&str) -> Option<String>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let path = var("PATH").unwrap_or_default();
    if let Some(bash) = on_path(&path, "bash.exe").find(|candidate| is_file(candidate)) {
        let normalized = bash.to_string_lossy().replace('\\', "/").to_lowercase();
        if !normalized.contains("/system32/bash.exe")
            && !normalized.contains("/windowsapps/bash.exe")
        {
            return Some(bash);
        }
    }
    if let Some(root) = on_path(&path, "git.exe")
        .find(|candidate| is_file(candidate))
        .and_then(|git| git.parent()?.parent().map(Path::to_path_buf))
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

/// PowerShell as Vibe 3.1.1 finds it when no shell is configured (`WindowsTerminalBackend.resolve_shell`): the first
/// of `pwsh.exe`, `powershell.exe` found on `PATH`. `var` reads an environment variable, `is_file` checks a path. Pure.
#[must_use]
pub fn powershell(
    var: impl Fn(&str) -> Option<String>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let path = var("PATH").unwrap_or_default();
    crate::powershell::FILE_NAMES
        .iter()
        .find_map(|name| on_path(&path, name).find(|candidate| is_file(candidate)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Forward slashes: Windows reads them, and `Path` splits them on every OS, so the tests hold everywhere.
    const GIT: &str = "C:/Program Files/Git";

    fn real_bash() -> PathBuf {
        Path::new(GIT).join("bin/bash.exe")
    }

    /// A Vibe CLI started in a terminal: Git for Windows' `cmd` folder on `PATH`, no `VW_GIT_BASH`.
    fn terminal_env(name: &str) -> Option<String> {
        match name {
            "PATH" => Some(format!("C:/Windows/System32;{GIT}/cmd")),
            "ProgramFiles" => Some("C:/Program Files".to_owned()),
            _ => None,
        }
    }

    fn git_installed(candidate: &Path) -> bool {
        *candidate == real_bash() || *candidate == Path::new(GIT).join("cmd/git.exe")
    }

    #[test]
    fn outside_the_app_the_bash_mode_finds_git_bash_as_vibe_does() {
        // NATIVEW0110B W-3: without VW_GIT_BASH the mode exited 127 and the CLI's strict hook denied every tool.
        let found = resolve_real(
            Kind::GitBash,
            crate::git_bash::ENV_REAL,
            terminal_env,
            git_installed,
        );
        assert_eq!(found, Some(real_bash()));
    }

    #[test]
    fn outside_the_app_the_powershell_mode_finds_powershell_on_path() {
        let v51 = r"C:\Windows\System32\WindowsPowerShell\v1.0";
        let var = |name: &str| (name == "PATH").then(|| v51.to_owned());
        let exe = Path::new(v51).join("powershell.exe");
        let found = resolve_real(
            Kind::PowerShell,
            crate::powershell::ENV_REAL,
            var,
            |candidate: &Path| *candidate == exe,
        );
        assert_eq!(found, Some(exe));
    }

    #[test]
    fn the_apps_variable_wins_and_a_bad_one_falls_back_to_discovery() {
        let own = if cfg!(windows) {
            r"D:\Tools\Git\bin\bash.exe"
        } else {
            "/opt/git/bin/bash.exe"
        };
        let with = |value: &'static str| {
            move |name: &str| match name {
                "VW_GIT_BASH" => Some(value.to_owned()),
                other => terminal_env(other),
            }
        };
        let both = |candidate: &Path| git_installed(candidate) || candidate == Path::new(own);
        assert_eq!(
            resolve_real(Kind::GitBash, "VW_GIT_BASH", with(own), both),
            Some(PathBuf::from(own)),
            "the app's own choice"
        );
        assert_eq!(
            resolve_real(Kind::GitBash, "VW_GIT_BASH", with("bash.exe"), both),
            Some(real_bash()),
            "a relative name is not trusted"
        );
    }

    #[test]
    fn nothing_found_is_none_and_this_program_is_never_chosen() {
        assert_eq!(
            resolve_real(Kind::GitBash, "VW_GIT_BASH", terminal_env, |_: &Path| false),
            None
        );
        // Our copy on PATH ahead of Git: skipped because `is_file` is false for this program.
        let copy = "C:/data/shell";
        let var = |name: &str| match name {
            "PATH" => Some(format!("{copy};{GIT}/cmd")),
            other => terminal_env(other),
        };
        let mine = Path::new(copy).join("bash.exe");
        let found = resolve_real(Kind::GitBash, "VW_GIT_BASH", var, |candidate: &Path| {
            *candidate != mine && git_installed(candidate)
        });
        assert_eq!(found, Some(real_bash()));
    }

    #[test]
    fn wsl_bash_and_the_store_alias_are_not_git_bash() {
        let var = |name: &str| {
            (name == "PATH").then(|| {
                r"C:\Windows\System32;C:\Users\u\AppData\Local\Microsoft\WindowsApps".to_owned()
            })
        };
        let found = git_bash(var, |candidate: &Path| {
            candidate.to_string_lossy().ends_with("bash.exe")
        });
        assert_eq!(found, None);
    }
}
