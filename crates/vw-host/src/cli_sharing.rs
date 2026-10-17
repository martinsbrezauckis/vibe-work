//! The "keep Vibe Work sessions separate from the CLI" switch as the Settings screen sees it:
//! the saved choice, whether a Vibe CLI home exists, and the CLI's version next to the bundled one.
//!
//! Depends on: `settings`, vw-supervisor (child environment filter), tokio (process, timeout).
//! Called by: [`crate::Host::cli_sharing`] / [`crate::Host::set_cli_sharing`], the app's `cli_sharing` command.
//! How the CLI is found: `vibe` (`vibe.exe` on Windows) in an absolute `PATH` entry or in a known user
//! folder (`~/.local/bin`, Homebrew, `/usr/local/bin`); a relative `PATH` entry is never searched (it would
//! resolve against whatever folder the app runs in). The version comes from `vibe --version`, run with the
//! same filtered environment the server gets, stdin closed, a short time limit and no output kept beyond
//! the version line.
//! Invariants:
//! - A CLI that is missing, slow or prints something unexpected means "version unknown"; the warning
//!   needs two known versions that differ, so unknown never warns.
//! - The switch applies the next time Vibe Work starts (the server's environment is fixed at start).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use vw_supervisor::{EnvPlan, build_env};

/// How long `vibe --version` may take (Python start-up on a cold disk can need a few seconds).
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(8);

/// Where to look for the CLI; the app fills it from the process environment, tests pass scratch folders.
#[derive(Debug, Clone, Default)]
pub struct CliProbe {
    /// The Vibe CLI's home (`$VIBE_HOME` or `~/.vibe`), if it could be resolved.
    pub cli_home: Option<PathBuf>,
    /// The value of `PATH`.
    pub path_var: Option<OsString>,
    /// The user's home folder (for `~/.local/bin`).
    pub home: Option<PathBuf>,
}

/// What the Settings row shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliSharingView {
    /// The saved choice: share the CLI's session folder (default on).
    pub share: bool,
    /// Whether the CLI's home folder exists (without it there is nothing to share yet).
    pub cli_home_found: bool,
    /// The CLI's version, when `vibe --version` answered.
    pub cli_version: Option<String>,
    /// The bundled server's version, when Vibe is running.
    pub bundled_version: Option<String>,
    /// Both versions are known and differ: show the one-line warning.
    pub version_mismatch: bool,
}

impl CliSharingView {
    /// Builds the view from its parts; the mismatch flag is derived here so it cannot disagree.
    #[must_use]
    pub fn new(
        share: bool,
        cli_home_found: bool,
        cli_version: Option<String>,
        bundled_version: Option<String>,
    ) -> Self {
        let version_mismatch =
            matches!((&cli_version, &bundled_version), (Some(a), Some(b)) if a != b);
        Self {
            share,
            cli_home_found,
            cli_version,
            bundled_version,
            version_mismatch,
        }
    }
}

/// The first absolute folder that holds the CLI executable.
fn find_cli(probe: &CliProbe) -> Option<PathBuf> {
    let name = if cfg!(windows) { "vibe.exe" } else { "vibe" };
    let mut dirs: Vec<PathBuf> = probe
        .path_var
        .as_deref()
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    if let Some(home) = probe.home.as_deref().filter(|h| h.is_absolute()) {
        dirs.push(home.join(".local").join("bin"));
    }
    if cfg!(not(windows)) {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    dirs.into_iter()
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|c| c.is_file())
}

/// The version in `vibe --version` output (`vibe 2.26.0`, `2.26.0`, `mistral-vibe 2.26.0 (abc)`): the first
/// word that starts with a digit and holds only digits, dots and an optional pre-release tail.
#[must_use]
pub fn parse_version(output: &str) -> Option<String> {
    output
        .lines()
        .next()?
        .split_whitespace()
        .map(|w| w.trim_start_matches('v'))
        .find(|w| {
            w.starts_with(|c: char| c.is_ascii_digit())
                && w.contains('.')
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        })
        .map(str::to_owned)
}

/// Runs `vibe --version` and returns the version, or `None` (no CLI, no answer in time, unexpected output).
pub async fn detect_cli_version(probe: &CliProbe, timeout: Duration) -> Option<String> {
    let exe = find_cli(probe)?;
    let fallback = probe.home.as_deref().map(|h| h.join(".vibe"));
    let home: &Path = probe.cli_home.as_deref().or(fallback.as_deref())?;
    let env = build_env(&EnvPlan {
        parent: std::env::vars_os().collect(),
        extra: &[],
        vibe_home: home,
        home: None,
        key: None,
        mcp_token: None,
    })
    .ok()?;
    let mut command = tokio::process::Command::new(exe);
    command
        .arg("--version")
        .env_clear()
        .envs(env)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true);
    // No console window for this probe (the app has no console of its own).
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&output.stdout))
}
