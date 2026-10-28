//! The `bash.exe` mode (Windows): Vibe Work's own Git Bash for Vibe (`[tools.git_bash] shell` in Vibe Work's
//! `VIBE_HOME/config.toml`, set by `vw-host`). It runs the real Git Bash named by [`ENV_REAL`] with Vibe's arguments
//! as written, inside a kill-on-close job (`vw_platform::kill_job`), and exits with its code. Vibe's Stop and its bash
//! timeout end this process; the job then ends every program the command started, also the ones a fork left without
//! a parent, which Vibe's own tree kill misses.
//!
//! The file must be named `bash.exe`: Vibe accepts a configured Git Bash shell only by that name.
//!
//! Depends on: `vw_platform::kill_job` (Windows). Called by: `cli::run` when the program is named `bash`.

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
    #[cfg(windows)]
    {
        let Some(real) = real_bash() else {
            eprintln!("Vibe Work: Git Bash was not found ({ENV_REAL} is not set to a bash.exe).");
            return ExitCode::from(NOT_FOUND);
        };
        match vw_platform::kill_job::run_in_kill_job(&real) {
            // Windows exit codes are 32 bits (an NTSTATUS such as 0xC000013A); passed on unchanged.
            Ok(code) => std::process::exit(i32::from_ne_bytes(code.to_ne_bytes())),
            Err(error) => {
                eprintln!("Vibe Work: Git Bash could not start: {error}");
                ExitCode::from(NOT_FOUND)
            }
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("Vibe Work: the {FILE_NAME} mode exists on Windows only.");
        ExitCode::from(NOT_FOUND)
    }
}

/// The real Git Bash from [`ENV_REAL`]: an absolute path to an existing file that is not this program (a copy of
/// this program there would start itself without end).
#[cfg(windows)]
fn real_bash() -> Option<std::path::PathBuf> {
    let real = std::path::PathBuf::from(std::env::var_os(ENV_REAL)?);
    if !real.is_absolute() || !real.is_file() {
        return None;
    }
    let me = std::env::current_exe().ok()?;
    let same = match (std::fs::canonicalize(&real), std::fs::canonicalize(&me)) {
        (Ok(a), Ok(b)) => a == b,
        _ => true,
    };
    (!same).then_some(real)
}
