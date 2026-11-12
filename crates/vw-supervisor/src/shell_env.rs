//! Prepares the app-owned startup file for agent Bash commands. Depends on std and config; called by the
//! launcher and environment builder. Only non-interactive child shells source this file.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// The startup file. `GIT_REDIRECT_STDIN` (`vw_agent_tools::agent_env::GIT_STDIN_VAR`): the Windows server environment
/// sets it to `NUL` against the Vibe 3.1.1 git start-up hang (`env::platform_vars`); with it, git would read an empty
/// input instead of a pipe the agent writes into it (`printf x | git hash-object --stdin`). Unset everywhere: no
/// effect where it is not set.
const STARTUP: &str = "# Agent command shells must report failures anywhere in a pipeline.\nset -o pipefail\n\
# The server's GIT_REDIRECT_STDIN=NUL (Windows) must not empty input the command pipes into git.\n\
unset GIT_REDIRECT_STDIN\n";

pub(crate) fn path(home: &Path) -> PathBuf {
    home.join("agent-shell.bash")
}

pub(crate) fn prepare(home: &Path) -> Result<(), crate::config::SupervisorError> {
    write(home).map_err(|source| crate::config::SupervisorError::ShellStartup {
        path: path(home),
        source,
    })
}

fn write(home: &Path) -> std::io::Result<()> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    std::fs::create_dir_all(home)?;
    let temp = home.join(format!(
        ".agent-shell-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        file.write_all(STARTUP.as_bytes())?;
        drop(file);
        // Replace the directory entry without following a planted startup-file symlink.
        std::fs::rename(&temp, path(home))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder for one test (removed at the end).
    fn scratch(case: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vw-shell-env-{case}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_startup_file_removes_the_servers_git_stdin_redirect() {
        let home = scratch("unset");
        prepare(&home).unwrap();
        let text = std::fs::read_to_string(path(&home)).unwrap();
        std::fs::remove_dir_all(&home).unwrap();
        assert!(text.contains("set -o pipefail\n"));
        assert!(
            text.lines()
                .any(|line| line == format!("unset {}", vw_agent_tools::agent_env::GIT_STDIN_VAR)),
            "startup file:\n{text}"
        );
    }

    /// The same as Vibe runs an agent command (`<bash> -c <command>`, `BASH_ENV` set by the builder): the variable
    /// the server has is gone inside the command.
    #[cfg(unix)]
    #[test]
    fn an_agent_bash_command_no_longer_sees_it() {
        let home = scratch("bash");
        prepare(&home).unwrap();
        let out = std::process::Command::new("/bin/bash")
            .args(["-c", "printf '%s' \"${GIT_REDIRECT_STDIN-unset}\""])
            .env("BASH_ENV", path(&home))
            .env(vw_agent_tools::agent_env::GIT_STDIN_VAR, "NUL")
            .output()
            .unwrap();
        std::fs::remove_dir_all(&home).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "unset");
    }
}
