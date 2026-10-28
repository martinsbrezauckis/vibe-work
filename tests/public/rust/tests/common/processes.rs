//! Disposable directories, process-table observations and bounded polling for integration tests.
//! Uses only the OS and supervisor state; starts no Vibe process itself.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use vw_supervisor::SupervisorState;

pub use vw_public_tests::test_paths;

/// Path of the probe helper executable.
pub fn probe_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_supervise_probe"))
}

/// A fresh scratch directory under the root bound into the real-server sandbox.
pub fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let base = test_paths::root();
    let dir = base.join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&base).unwrap();
    vw_platform::remove_owned_tree(&base, &dir).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.canonicalize().unwrap();
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// True while `pid` is a live (non-zombie) process.
pub fn alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let state = stat
            .rsplit_once(") ")
            .and_then(|(_, rest)| rest.chars().next());
        state != Some('Z') && state != Some('X')
    }
    #[cfg(target_os = "macos")]
    {
        // `ps` state: empty when gone, starting with `Z` for an unreaped zombie.
        let out = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        !state.is_empty() && !state.starts_with('Z')
    }
    #[cfg(windows)]
    {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
    }
}

/// True if `pid` exists as an unreaped zombie (Linux only; false elsewhere).
pub fn zombie(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit_once(") ")
                    .and_then(|(_, rest)| rest.chars().next())
            })
            == Some('Z')
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

/// Polls `cond` every 20 ms until true or `limit` passes; returns whether it became true.
///
/// the fixture: `limit` is the quiet-machine budget; the poll waits six times as long before it gives up, so a machine
/// busy with other builds cannot turn a slow-but-correct run into a failure. A passing run returns as soon as
/// `cond` holds, so only a real failure pays the longer wait.
pub async fn eventually(limit: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let limit = limit * 6;
    let started = Instant::now();
    while started.elapsed() < limit {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cond()
}

/// Waits until the state matches `pred` (or panics after `limit`), returning that state.
pub async fn wait_state(
    rx: &mut tokio::sync::watch::Receiver<SupervisorState>,
    limit: Duration,
    pred: impl Fn(&SupervisorState) -> bool,
) -> SupervisorState {
    let matched = match tokio::time::timeout(limit, rx.wait_for(|s| pred(s))).await {
        Ok(Ok(state)) => Some(state.clone()),
        _ => None,
    };
    matched.unwrap_or_else(|| {
        panic!(
            "state did not match within {limit:?}; last: {:?}",
            *rx.borrow()
        )
    })
}

/// Kills a process hard (SIGKILL / TerminateProcess). One that is already gone counts as killed: on Windows a server
/// lives in its owner's kill-on-close job and dies with that owner.
pub fn kill_hard(pid: u32) {
    #[cfg(unix)]
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status();
    #[cfg(windows)]
    let status = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .status();
    assert!(
        status.unwrap().success() || !alive(pid),
        "could not kill {pid}"
    );
}
