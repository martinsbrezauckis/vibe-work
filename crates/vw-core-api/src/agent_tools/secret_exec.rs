//! Runs ONE program for the secret broker (`secret_run`): the agent process environment plus
//! the secrets in their variables, stdin closed, output captured with a cap, a time limit that stops the whole tree.
//! ShellX Vault's `run_approved_agent_command_with_capture` (`tauri-plugin-vault/src/commands.rs:512-640` @ ded9de1)
//! as Vibe Work's own code; changed: the process is registered with the process monitor as the session's agent
//! process (it shows in the session's processes and is stopped as a tree on time out), and output is withheld by the
//! agent tools listener's scrub (`server.rs`) instead of an exact-match replace here (more forms are covered).
//!
//! On Linux the program starts through the descriptor the final check hashed,
//! (`/proc/self/fd/<n>`, `argv[0]` = its real path), so what runs is the file that was checked even if another file
//! is renamed over the path meanwhile; a `#!` program, other systems, or a missing `/proc` run by path
//! (`secret_pin` module header).
//!
//! Depends on: tokio (`process`), vw-procmon, vw-vault (`SecretString`). Called by: `secret_run`.
//! Invariants: the values go only into the child's environment (never argv, never a log); the environment is cleared
//! first; a variable the allowlist also has is replaced by the secret, not doubled.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use vw_procmon::{OwnProcess, ProcessClass, ProcessGroup, ProcessRef};
use vw_vault::SecretString;

use crate::core::Shared;

/// Output kept per stream, bytes.
pub(crate) const OUTPUT_CAP: usize = 32 * 1024;
/// The process monitor label of a broker run (never the command).
const LABEL: &str = "secret_run";

/// What to run.
pub(crate) struct Exec<'a> {
    /// The agent process environment allowlist.
    pub(crate) process_env: &'a [(OsString, OsString)],
    /// The session (process attribution).
    pub(crate) session_id: &'a str,
    /// The resolved program.
    pub(crate) program: &'a Path,
    /// The pinned files as the final check left them open (the program is started through it on Linux).
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "the caller keeps the files open; programs run by path outside Linux"
        )
    )]
    pub(crate) held: &'a super::secret_pin::Held,
    /// Its arguments.
    pub(crate) args: &'a [String],
    /// Working folder.
    pub(crate) cwd: &'a Path,
    /// Time limit.
    pub(crate) timeout: Duration,
}

/// Runs `exec` with `values` (variable, value) in its environment; answers exit code, output and flags.
///
/// # Errors
/// The program could not be started.
pub(crate) async fn execute(
    shared: &Arc<Shared>,
    exec: &Exec<'_>,
    values: &[(String, SecretString)],
) -> Result<Value, String> {
    let mut command = command_for(exec);
    command
        .args(exec.args)
        .current_dir(exec.cwd)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut env: Vec<(OsString, OsString)> = exec
        .process_env
        .iter()
        .filter(|(name, _)| {
            !values
                .iter()
                .any(|(secret_name, _)| name.to_string_lossy().eq_ignore_ascii_case(secret_name))
        })
        .cloned()
        .collect();
    env.extend(
        values
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value.expose()))),
    );
    vw_supervisor::isolate_credentials(&mut env);
    command.envs(env);
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000 | 0x0000_0200); // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
    let mut child = command
        .spawn()
        .map_err(|e| format!("Not run: the program could not be started ({e})"))?;
    let process = child.id().and_then(vw_procmon::identity);
    if let Some(process) = process {
        register(shared, exec.session_id, process, child.id());
    }
    let out = tokio::spawn(drain(child.stdout.take()));
    let err = tokio::spawn(drain(child.stderr.take()));
    let waited = tokio::time::timeout(exec.timeout, child.wait()).await;
    let timed_out = waited.is_err();
    let status = if let Ok(status) = waited {
        status.ok()
    } else {
        stop_tree(shared, exec.session_id, process).await;
        let _ = child.start_kill();
        child.wait().await.ok()
    };
    if let Some(process) = process {
        shared.procmon.registry().unregister_process(process);
    }
    let (stdout, out_cut) = finish(out).await;
    let (stderr, err_cut) = finish(err).await;
    Ok(json!({
        "exitCode": status.and_then(|s| s.code()),
        "success": !timed_out && status.is_some_and(|s| s.success()),
        "timedOut": timed_out,
        "stdout": String::from_utf8_lossy(&stdout),
        "stderr": String::from_utf8_lossy(&stderr),
        "outputTruncated": out_cut || err_cut,
    }))
}

/// The command that starts the checked program: through its held descriptor on Linux (see the module header), by path
/// otherwise.
fn command_for(exec: &Exec<'_>) -> tokio::process::Command {
    #[cfg(target_os = "linux")]
    if !exec.held.script && Path::new("/proc/self/fd").is_dir() {
        use std::os::fd::AsRawFd as _;
        // The descriptor closes at exec (std opens files close-on-exec); the kernel resolves this path in the child
        // before that, to the inode the check hashed. `/proc/self/exe` of the program is then its real path, so
        // programs that find their own files next to themselves keep working.
        let mut command = tokio::process::Command::new(format!(
            "/proc/self/fd/{}",
            exec.held.program.as_raw_fd()
        ));
        command.arg0(exec.program);
        return command;
    }
    tokio::process::Command::new(exec.program)
}

fn register(shared: &Shared, session_id: &str, process: ProcessRef, pid: Option<u32>) {
    let group = if cfg!(unix) {
        pid.map_or(ProcessGroup::None, |pgid| ProcessGroup::Unix { pgid })
    } else {
        ProcessGroup::None
    };
    let _ = shared.procmon.registry().register(OwnProcess {
        session_id: Some(session_id.to_owned()),
        process,
        group,
        class: ProcessClass::Agent,
        label: LABEL.to_owned(),
        keep_running: false,
    });
}

/// Stops the run's process tree (time limit): the monitor's safe stop for this session.
async fn stop_tree(shared: &Arc<Shared>, session_id: &str, process: Option<ProcessRef>) {
    let Some(process) = process else {
        return;
    };
    let (shared, session) = (shared.clone(), session_id.to_owned());
    let _ = tokio::task::spawn_blocking(move || {
        let sessions = crate::processes::contexts(&shared.store);
        shared.procmon.stop(&session, process, &sessions)
    })
    .await;
}

/// Reads a stream to its end, keeping at most [`OUTPUT_CAP`] bytes. Returns the bytes and whether more came.
async fn drain<R: tokio::io::AsyncRead + Unpin>(reader: Option<R>) -> (Vec<u8>, bool) {
    let Some(mut reader) = reader else {
        return (Vec::new(), false);
    };
    let mut kept = Vec::new();
    let mut cut = false;
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = OUTPUT_CAP.saturating_sub(kept.len());
                kept.extend_from_slice(&buffer[..n.min(room)]);
                cut |= n > room;
            }
        }
    }
    (kept, cut)
}

/// The drained output, waiting at most 2 s after the program ended (a child it left behind may hold the pipe).
async fn finish(task: tokio::task::JoinHandle<(Vec<u8>, bool)>) -> (Vec<u8>, bool) {
    let mut task = task;
    match tokio::time::timeout(Duration::from_secs(2), &mut task).await {
        Ok(Ok(output)) => output,
        Ok(Err(_)) => (Vec::new(), false),
        Err(_) => {
            task.abort();
            (Vec::new(), true)
        }
    }
}
