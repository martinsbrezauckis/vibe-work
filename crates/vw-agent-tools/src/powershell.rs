//! The PowerShell mode (Windows): Vibe Work's own PowerShell for Vibe (`[tools.powershell] shell` in Vibe Work's
//! `VIBE_HOME/config.toml`, set by `vw-host` when Vibe runs the agent's commands in PowerShell, which Vibe 3.1.1 does
//! when it finds no Git Bash). It runs the real PowerShell named by [`ENV_REAL`] with Vibe's arguments as written,
//! inside a kill-on-close job and without the server's `GIT_REDIRECT_STDIN`, exactly as the `bash.exe` mode does
//! ([`crate::git_bash::run_real`]). Vibe starts PowerShell with `-NoProfile`, so the wrapper is the one place where Vibe
//! Work can remove that variable before the agent's command runs.
//!
//! Vibe accepts a configured PowerShell only by the names in [`FILE_NAMES`] (Vibe 3.1.1
//! `_processes/_windows.py` `_shell_family`); `vw-host` gives the copy the real program's own name.
//!
//! Depends on: [`crate::git_bash`]. Called by: `cli::run` when the program is named `pwsh` or `powershell`.

use std::process::ExitCode;

/// The variable that names the real PowerShell (`vw-host` sets it in the Vibe server's environment).
pub const ENV_REAL: &str = "VW_POWERSHELL";

/// The program names Vibe accepts for a configured PowerShell, in Vibe's own search order (PowerShell 7 first).
pub const FILE_NAMES: [&str; 2] = ["pwsh.exe", "powershell.exe"];

/// Runs the real PowerShell in a kill-on-close job and exits with its exit code.
#[must_use]
pub fn run() -> ExitCode {
    crate::git_bash::run_real(crate::shell_find::Kind::PowerShell, ENV_REAL, "PowerShell")
}

/// True when this program was started under a PowerShell name (file stem `pwsh` or `powershell`, any case).
#[must_use]
pub fn is_mode_name(stem: &std::ffi::OsStr) -> bool {
    FILE_NAMES.iter().any(|name| {
        name.strip_suffix(".exe")
            .is_some_and(|wanted| stem.eq_ignore_ascii_case(wanted))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn both_powershell_names_select_the_mode_and_nothing_else_does() {
        for stem in ["pwsh", "PWSH", "powershell", "PowerShell"] {
            assert!(is_mode_name(OsStr::new(stem)), "{stem}");
        }
        for stem in ["bash", "pwsh7", "vw_agent_tools", "powershell_ise", ""] {
            assert!(!is_mode_name(OsStr::new(stem)), "{stem}");
        }
    }
}
