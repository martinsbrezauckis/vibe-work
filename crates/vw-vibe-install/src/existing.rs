//! "Use my existing install": finds `vibe-app-server` executables the user already has (PATH and
//! the known install locations, through vw-supervisor's discovery: absolute, canonical, `.exe` only on
//! Windows) and reads each one's version from its `initialize` answer (2.26.0 has no `--version`).
//!
//! The probe runs the user's binary for a few seconds with a throwaway `VIBE_HOME` and home under `probe_dir`
//! (never `~/.vibe`), no key, the allowlisted environment, and stdin closed after the answer.
//!
//! Depends on: vw-supervisor (discovery, `default_initialize_params`), tokio. Called by: [`crate::manager`].

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use vw_supervisor::{Discovery, canonical_executable};

use crate::view::ExistingView;

/// Longest a probe may take.
pub const PROBE_LIMIT: Duration = Duration::from_secs(25);
/// Longest the start check of the person's remembered install may take: the first start of a Python environment
/// after a reboot (cold disk cache, a virus scan of every file) can take longer than a search probe's limit, and a
/// remembered install that answers late must not be reported as one that does not answer.
pub const SAVED_PROBE_LIMIT: Duration = Duration::from_secs(60);
/// Candidates probed at most.
pub const MAX_PROBES: usize = 4;

/// The reason an install of another version cannot be chosen.
#[must_use]
pub fn version_warning(found: &str, pinned: &str) -> String {
    format!("This install is Vibe {found}; Vibe Work needs Vibe {pinned}.")
}

/// True when the canonical candidate `path` lies in Vibe Work's own install folder `own`, which is skipped (it is
/// not the user's existing install).
///
/// `path` comes from `canonical_executable` (links resolved; on Windows long names and the on-disk case, without the
/// verbatim prefix), while `own` is the folder as configured: through a symlinked folder, or on Windows in another
/// spelling, the plain comparison misses. Then both are compared in their canonical forms (`canonicalize`); a side
/// that cannot be canonicalized compares as written only.
fn in_own_install(path: &Path, own: &Path) -> bool {
    path.starts_with(own)
        || matches!(
            (path.canonicalize(), own.canonicalize()),
            (Ok(path), Ok(own)) if path.starts_with(&own)
        )
}

/// Every distinct existing server, with its version (or `None` when it did not answer).
pub async fn find(
    discovery: &Discovery,
    probe_dir: &Path,
    parent_env: &[(OsString, OsString)],
    pinned: &str,
    exclude: &Path,
) -> Vec<ExistingView> {
    let mut seen: Vec<PathBuf> = Vec::new();
    for candidate in discovery.candidates() {
        if let Some(path) = canonical_executable(&candidate)
            && !in_own_install(&path, exclude)
            && !seen.contains(&path)
        {
            seen.push(path);
        }
        if seen.len() == MAX_PROBES {
            break;
        }
    }
    let mut out = Vec::new();
    for (index, path) in seen.into_iter().enumerate() {
        let home = probe_dir.join(format!("probe-{index}"));
        let version = probe(&path, &home, parent_env).await;
        let supported = version.as_deref() == Some(pinned);
        let warning = match &version {
            Some(v) if v != pinned => Some(version_warning(v, pinned)),
            None => Some(NOT_AN_APP_SERVER.to_owned()),
            _ => None,
        };
        out.push(ExistingView {
            path: path.display().to_string(),
            version,
            supported,
            warning,
        });
    }
    out
}

/// Starts `program`, sends `initialize`, returns `serverInfo.version`. `None` on any failure or timeout.
pub async fn probe(
    program: &Path,
    home: &Path,
    parent_env: &[(OsString, OsString)],
) -> Option<String> {
    probe_with_args(program, &[], home, parent_env).await
}

/// The same bounded initialize probe through a development recording wrapper's arguments.
/// The wrapper must transparently launch the real server; it never supplies a version itself.
pub async fn probe_with_args(
    program: &Path,
    args: &[OsString],
    home: &Path,
    parent_env: &[(OsString, OsString)],
) -> Option<String> {
    probe_within(PROBE_LIMIT, program, args, home, parent_env).await
}

/// [`probe_with_args`] with its own time limit.
pub async fn probe_within(
    limit: Duration,
    program: &Path,
    args: &[OsString],
    home: &Path,
    parent_env: &[(OsString, OsString)],
) -> Option<String> {
    probe_checked(limit, program, args, home, parent_env)
        .await
        .ok()
}

/// Why a probe produced no version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeFailure {
    /// The program started and stayed silent for the whole limit.
    TimedOut,
    /// The program could not be started (or its stdin could not be written).
    Spawn(String),
    /// The program ended before it answered: its exit code. Its error output is never kept: the probe passes the
    /// supervisor's environment allowlist (proxy URLs may carry a password) and a failing program may print it.
    Exited {
        /// The exit code, when the OS reported one.
        code: Option<i32>,
    },
    /// The program answered, but not with a Vibe app server's `serverInfo.version`.
    NoVersion,
}

/// [`probe_within`] with the reason of a failure, so a program that ended is told apart from one that stayed silent.
///
/// # Errors
/// The [`ProbeFailure`] that ended the probe.
pub async fn probe_checked(
    limit: Duration,
    program: &Path,
    args: &[OsString],
    home: &Path,
    parent_env: &[(OsString, OsString)],
) -> Result<String, ProbeFailure> {
    tokio::time::timeout(limit, probe_inner(program, args, home, parent_env))
        .await
        .unwrap_or(Err(ProbeFailure::TimedOut))
}

/// The start check's warning for a remembered install: `None` when it is the `pinned` version. Only a real silence
/// gets the timeout words; any other failure uses the search's wording for a program that is not Vibe's app server.
#[must_use]
pub fn saved_warning(
    result: &Result<String, ProbeFailure>,
    limit: Duration,
    pinned: &str,
) -> Option<String> {
    match result {
        Ok(found) if found == pinned => None,
        Ok(found) => Some(version_warning(found, pinned)),
        Err(ProbeFailure::TimedOut) => Some(no_answer_warning(limit, pinned)),
        Err(_) => Some(NOT_AN_APP_SERVER.to_owned()),
    }
}

/// The words for a program that ended or answered without being Vibe's app server.
const NOT_AN_APP_SERVER: &str =
    "This program did not answer like Vibe's app server; it cannot be used.";

/// The start check's words for a remembered install that did not answer within `limit`: what happened and what
/// the person can do, without claiming a version it did not report.
#[must_use]
pub fn no_answer_warning(limit: Duration, pinned: &str) -> String {
    format!(
        "This install did not answer within {} seconds. Start Vibe Work again to check it once more, or choose \
         another; Vibe Work needs Vibe {pinned}.",
        limit.as_secs()
    )
}

async fn probe_inner(
    program: &Path,
    args: &[OsString],
    home: &Path,
    parent_env: &[(OsString, OsString)],
) -> Result<String, ProbeFailure> {
    let spawn_failure = |error: &dyn std::fmt::Display| ProbeFailure::Spawn(error.to_string());
    let vibe_home = home.join("vibe-home");
    std::fs::create_dir_all(&vibe_home).map_err(|e| spawn_failure(&e))?;
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let env = crate::steps::environment(
        parent_env.iter().cloned(),
        &[
            ("VIBE_HOME".to_owned(), vibe_home.into_os_string()),
            (home_var.to_owned(), home.as_os_str().to_owned()),
            ("PYTHONUNBUFFERED".to_owned(), OsString::from("1")),
        ],
    );
    let mut command = vw_platform::process_command::hidden_tokio(program);
    command
        .args(args)
        .env_clear()
        .envs(env)
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = crate::steps::spawn_with_retry(|| command.spawn())
        .await
        .map_err(|e| spawn_failure(&e))?;
    let params = serde_json::to_value(vw_supervisor::default_initialize_params())
        .map_err(|e| spawn_failure(&e))?;
    let request =
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": params});
    let mut stdin = child.stdin.take().ok_or(ProbeFailure::NoVersion)?;
    let wrote = async {
        stdin.write_all(format!("{request}\n").as_bytes()).await?;
        stdin.flush().await
    }
    .await;
    let stdout = child.stdout.take().ok_or(ProbeFailure::NoVersion)?;
    let mut lines = BufReader::new(stdout).lines();
    let mut answered = false;
    let mut version = None;
    let mut request_id = 1;
    if wrote.is_ok() {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(frame) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if frame.get("id") == Some(&serde_json::json!(request_id)) {
                // Older servers reject capabilities added after their release. Retry once, on that explicit
                // protocol error only, to learn the version; the caller still requires the exact pinned version.
                if request_id == 1
                    && matches!(frame.pointer("/error/code"), Some(code) if code == "invalid_params" || code == -32602)
                {
                    request_id = 2;
                    let minimal = serde_json::json!({
                        "jsonrpc": "2.0", "id": request_id, "method": "initialize",
                        "params": {
                            "clientInfo": {"name": "vibe-work", "version": env!("CARGO_PKG_VERSION")},
                            "capabilities": {"callbackKinds": ["approval", "user_input"]}
                        }
                    });
                    if stdin
                        .write_all(format!("{minimal}\n").as_bytes())
                        .await
                        .is_err()
                        || stdin.flush().await.is_err()
                    {
                        break;
                    }
                    continue;
                }
                answered = true;
                version = frame
                    .pointer("/result/serverInfo/version")
                    .and_then(|v| v.as_str())
                    .filter(|v| !v.trim().is_empty())
                    .map(str::to_owned);
                break;
            }
        }
    }
    drop(stdin);
    let waited = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    let status = if let Ok(status) = waited {
        status.ok()
    } else {
        let _ = child.kill().await;
        None
    };
    if let Some(version) = version {
        return Ok(version);
    }
    if answered {
        return Err(ProbeFailure::NoVersion);
    }
    Err(ProbeFailure::Exited {
        code: status.and_then(|s| s.code()),
    })
}

#[cfg(test)]
#[path = "existing_tests.rs"]
mod tests;
