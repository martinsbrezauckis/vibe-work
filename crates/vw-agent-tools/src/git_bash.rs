//! The `bash.exe` mode (Windows): Vibe Work's own Git Bash for Vibe (`[tools.git_bash] shell` in Vibe Work's
//! `VIBE_HOME/config.toml`, set by `vw-host`). It runs the real Git Bash named by [`ENV_REAL`] with Vibe's arguments
//! as written, inside a kill-on-close job (`vw_platform::kill_job`), and exits with its code. Vibe's Stop and its bash
//! timeout end this process; the job then ends every program the command started, also the ones a fork left without
//! a parent, which Vibe's own tree kill misses.
//!
//! A Vibe CLI the person starts in a terminal with Vibe Work's `VIBE_HOME` reads the same `shell` line but has no
//! [`ENV_REAL`]; the mode then runs the Git Bash Vibe itself would find ([`crate::shell_find`]), so the CLI's commands
//! and its strict kill-guard hook (run through this shell) work instead of failing with 127.
//!
//! The file must be named `bash.exe`: Vibe accepts a configured Git Bash shell only by that name.
//!
//! The real shell starts without the server's `GIT_REDIRECT_STDIN` ([`crate::agent_env::GIT_STDIN_VAR`]), so input a
//! command pipes into git reaches git (the Git Bash startup file also unsets it; this covers the PowerShell mode,
//! [`crate::powershell`], which shares [`run_real`]).
//!
//! Depends on: `vw_platform::kill_job` (Windows), [`crate::shell_find`]. Called by: `cli::run` when the program is named `bash`.

use std::process::ExitCode;

/// The variable that names the real Git Bash (`vw-host` sets it in the Vibe server's environment).
pub const ENV_REAL: &str = "VW_GIT_BASH";

/// The file name Vibe requires of a configured Git Bash shell.
pub const FILE_NAME: &str = "bash.exe";

/// The exit code for a missing or unusable real Git Bash (bash's "command not found").
const NOT_FOUND: u8 = 127;

/// Runs the real Git Bash in a kill-on-close job and exits with its exit code.
#[must_use]
pub fn run() -> ExitCode {
    run_real(crate::shell_find::Kind::GitBash, ENV_REAL, "Git Bash")
}

/// Runs the real `label` shell (named by `env_real`, else found as Vibe finds it: [`crate::shell_find::resolve_real`])
/// in a kill-on-close job without the server's `GIT_REDIRECT_STDIN`, and exits with its exit code.
#[must_use]
pub(crate) fn run_real(kind: crate::shell_find::Kind, env_real: &str, label: &str) -> ExitCode {
    #[cfg(windows)]
    {
        let Some(real) = real_program(kind, env_real) else {
            eprintln!(
                "Vibe Work: {label} was not found ({env_real} is not set and Vibe would find none)."
            );
            return ExitCode::from(NOT_FOUND);
        };
        match vw_platform::kill_job::run_in_kill_job(&real, &[crate::agent_env::GIT_STDIN_VAR]) {
            // Windows exit codes are 32 bits (an NTSTATUS such as 0xC000013A); passed on unchanged.
            Ok(code) => std::process::exit(i32::from_ne_bytes(code.to_ne_bytes())),
            Err(error) => {
                eprintln!("Vibe Work: {label} could not start: {error}");
                ExitCode::from(NOT_FOUND)
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (kind, env_real);
        eprintln!("Vibe Work: the {label} mode exists on Windows only.");
        ExitCode::from(NOT_FOUND)
    }
}

/// The real shell: `env_real`'s program, else the one Vibe would find (a Vibe CLI started outside Vibe Work reads our
/// `shell` line without the app's variable). Never this program (a copy of it would start itself without end).
#[cfg(windows)]
fn real_program(kind: crate::shell_find::Kind, env_real: &str) -> Option<std::path::PathBuf> {
    let me = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    let usable = |candidate: &std::path::Path| {
        candidate.is_file() && std::fs::canonicalize(candidate).is_ok_and(|real| real != me)
    };
    crate::shell_find::resolve_real(kind, env_real, |name| std::env::var(name).ok(), usable)
}
