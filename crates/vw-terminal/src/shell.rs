//! Which shell a terminal starts: the user's default shell per platform.
//!
//! Depends on: std. Called by: `manager` (when the config names no shell), tests.
//! Rules:
//! - Unix: `$SHELL` when it is an absolute path to an executable file, else `/bin/sh`; started as a
//!   login shell (`-l`) for shells known to take it, so rc/profile files restore the user's PATH.
//!   ShellX fell back to `/bin/bash` (`terminal.rs:340-343`).
//! - Windows: `pwsh.exe` found on `PATH` (absolute result), else Windows PowerShell under
//!   `%SystemRoot%`, else `%ComSpec%`, else `cmd.exe`. ShellX spawned `where` to look (`:393-402`);
//!   the search here is in-process. PowerShell starts with `-NoLogo` (its profile still runs).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// A program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCommand {
    /// The program (absolute when chosen by [`default_shell`]).
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<OsString>,
}

impl ShellCommand {
    /// A program with arguments.
    #[must_use]
    pub fn new(program: impl Into<PathBuf>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(OsString::from).collect(),
        }
    }
}

/// Unix shells that accept `-l` (login shell).
#[cfg(unix)]
const LOGIN_CAPABLE: &[&str] = &[
    "bash", "zsh", "fish", "sh", "dash", "ksh", "mksh", "tcsh", "csh", "nu", "elvish",
];

fn lookup<'a>(env: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsStr> {
    env.iter()
        .find(|(k, _)| {
            if cfg!(windows) {
                k.to_string_lossy().eq_ignore_ascii_case(name)
            } else {
                k == name
            }
        })
        .map(|(_, v)| v.as_os_str())
}

/// The user's default shell, chosen from the terminal's environment (`env`, already filtered).
#[must_use]
pub fn default_shell(env: &[(OsString, OsString)]) -> ShellCommand {
    platform_shell(env)
}

#[cfg(unix)]
fn platform_shell(env: &[(OsString, OsString)]) -> ShellCommand {
    let program = lookup(env, "SHELL")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && is_executable(p))
        .unwrap_or_else(|| PathBuf::from("/bin/sh"));
    let login = program
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| LOGIN_CAPABLE.contains(&name));
    ShellCommand {
        program,
        args: if login {
            vec![OsString::from("-l")]
        } else {
            Vec::new()
        },
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn platform_shell(env: &[(OsString, OsString)]) -> ShellCommand {
    let on_path = |exe: &str| -> Option<PathBuf> {
        let path = lookup(env, "PATH")?;
        std::env::split_paths(path)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(exe))
            .find(|candidate| candidate.is_file())
    };
    if let Some(pwsh) = on_path("pwsh.exe") {
        return ShellCommand {
            program: pwsh,
            args: vec![OsString::from("-NoLogo")],
        };
    }
    if let Some(root) = lookup(env, "SystemRoot") {
        let powershell = Path::new(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        if powershell.is_file() {
            return ShellCommand {
                program: powershell,
                args: vec![OsString::from("-NoLogo")],
            };
        }
    }
    let comspec = lookup(env, "ComSpec")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file())
        .unwrap_or_else(|| PathBuf::from("cmd.exe"));
    ShellCommand {
        program: comspec,
        args: Vec::new(),
    }
}
