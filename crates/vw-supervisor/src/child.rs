//! Spawns the server process and owns it: exit watch, tree kill, reaping.
//!
//! Depends on: `process-wrap` (process group on Unix, kill-on-close job object on Windows),
//! tokio. Called by: `supervisor` (one [`ServerChild`] per server start).
//! Invariants:
//! - The server is a process-group leader (Unix) or alone in its own kill-on-close job object
//!   (Windows, created suspended and assigned by handle, then resumed: no PID race, unlike ShellX
//!   `winproc.rs:330-342 @ e38c2b5c`). Killing means killing the whole tree: Vibe runs agent tools
//!   (`bash`) as its children, and ShellX `eab16b7a` showed descendants outliving a root-only kill.
//! - When the leader exits on its own, the rest of its tree is killed too (tools of a dead server
//!   are orphans nobody manages), and everything is reaped: no zombies.
//! - If the owner is dropped without `shutdown`, the tree is still killed (`TreeGuard`), and on
//!   Windows the job handle closing kills it even if this process is killed hard.
//! - Unix has no equivalent of the job object for "parent killed with SIGKILL". There the server
//!   loses its stdin and `vibe-app-server` shuts down gracefully on stdin EOF, and a
//!   watchdog (block `contain`, note section 10) covers what that leaves: one `/bin/sh` per server
//!   start, in its own process group, named `vw-server-watchdog`, reading a pipe this process holds and
//!   never writes. When this process dies the read ends; the watchdog waits `watchdog_grace` for the
//!   graceful exit, then `kill -KILL -<pgid>` if the server's process group still has a member (a group
//!   id stays allocated while any member lives, so it is the group our server created). Processes Vibe
//!   starts for the agent `setsid` into other groups and are not touched. Not used:
//!   `PR_SET_PDEATHSIG` (a signal at the crash skips Vibe's graceful cleanup, b; it also fires when
//!   the spawning *thread* exits) and `PR_SET_CHILD_SUBREAPER` (reaping orphans would race tokio's own
//!   child handling). On a normal stop or restart the watchdog is killed with the handle.
//! - The leader's and the watchdog's ids are recorded in the supervisor's `LiveIds` while they are unreaped
//!   children, for the synchronous last-resort stop at quit (`Supervisor::kill_now`).
//! - The same `spawn` is the path for any other long-lived helper this core starts itself. A stdio MCP
//!   server that Vibe starts is Vibe's child and must exit on stdin EOF.
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{debug, warn};
use vw_rpc::ExitInfo;

use crate::live_ids::LiveIds;

/// What to start.
pub(crate) struct Launch {
    /// argv[0]: the launcher (e.g. bwrap) or the server itself.
    pub(crate) program: OsString,
    /// The remaining argv.
    pub(crate) args: Vec<OsString>,
    /// The complete environment (the child starts from an empty one).
    pub(crate) env: Vec<(OsString, OsString)>,
    /// Working directory.
    pub(crate) cwd: PathBuf,
    /// Unix crash watchdog grace (`None`: no watchdog).
    pub(crate) watchdog_grace: Option<Duration>,
}

/// The spawned server's pipes.
pub(crate) struct Pipes {
    pub(crate) stdin: ChildStdin,
    pub(crate) stdout: ChildStdout,
    pub(crate) stderr: ChildStderr,
}

/// Handle to the task that owns the server process.
pub(crate) struct ServerChild {
    pid: Option<u32>,
    /// Unix crash watchdog; killed when this handle drops (normal stop or restart).
    _watchdog: Option<Watchdog>,
    kill_tx: mpsc::Sender<()>,
    exit_rx: watch::Receiver<Option<ExitInfo>>,
    task: JoinHandle<()>,
    ids: LiveIds,
}

impl Drop for ServerChild {
    /// Dropping the handle without `kill_and_reap` aborts the owner task, whose `TreeGuard` then
    /// kills the tree: the server never outlives its supervisor. Reaping is then tokio's (its orphan queue), so the
    /// leader's id leaves `LiveIds` here.
    fn drop(&mut self) {
        self.task.abort();
        self.ids.leader_gone(self.pid);
    }
}

/// The Unix crash watchdog process (see the module doc). Holds the write end of its stdin; dropping it
/// kills the watchdog (it must not act on a deliberate stop) and reaps it, its id cleared from `LiveIds` first.
struct Watchdog(std::process::Child, LiveIds);

impl Drop for Watchdog {
    fn drop(&mut self) {
        let Self(child, ids) = self;
        ids.watchdog_gone(child.id(), || {
            let _ = child.kill();
            let _ = child.wait();
        });
    }
}

/// The watchdog's script: `$1` = the server's process group, `$2` = grace in seconds.
#[cfg(unix)]
const WATCHDOG_SCRIPT: &str =
    r#"read -r _; sleep "$2"; if kill -0 -"$1" 2>/dev/null; then kill -KILL -"$1"; fi"#;

/// Starts the watchdog for the process group `leader` (Unix). A failure is logged, not fatal: the next
/// start's ledger sweep still covers a crash.
#[cfg(unix)]
fn start_watchdog(leader: Option<u32>, grace: Option<Duration>, ids: &LiveIds) -> Option<Watchdog> {
    use std::os::unix::process::CommandExt;
    let (leader, grace) = (leader?, grace?);
    let started = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(WATCHDOG_SCRIPT)
        .arg("vw-server-watchdog")
        .arg(leader.to_string())
        .arg(format!("{:.3}", grace.as_secs_f64()))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    match started {
        Ok(child) => {
            ids.set_watchdog(Some(child.id()));
            Some(Watchdog(child, ids.clone()))
        }
        Err(error) => {
            warn!(%error, "crash watchdog not started; the ledger sweep remains");
            None
        }
    }
}

#[cfg(not(unix))]
fn start_watchdog(
    _leader: Option<u32>,
    _grace: Option<Duration>,
    _ids: &LiveIds,
) -> Option<Watchdog> {
    None
}

/// Kills the tree when the owning task is dropped or aborted without a normal end.
struct TreeGuard(Box<dyn ChildWrapper>);

impl Drop for TreeGuard {
    fn drop(&mut self) {
        // Errors mean "already gone"; nothing else can be done in a destructor.
        let _ = self.0.start_kill();
    }
}

/// Spawns the process and its owner task; records the leader's and the watchdog's ids in `ids`.
pub(crate) fn spawn(launch: &Launch, ids: &LiveIds) -> std::io::Result<(ServerChild, Pipes)> {
    let mut command = CommandWrap::with_new(&launch.program, |cmd| {
        cmd.args(&launch.args)
            .env_clear()
            .envs(
                launch
                    .env
                    .iter()
                    .map(|(k, v)| (k.as_os_str(), v.as_os_str())),
            )
            .current_dir(&launch.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    });
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    {
        // No console window per spawn (ShellX winproc.rs:27-58 scar), then the job object.
        command.wrap(process_wrap::tokio::CreationFlags(
            windows::Win32::System::Threading::CREATE_NO_WINDOW,
        ));
        command.wrap(process_wrap::tokio::JobObject);
    }
    // Last resort for the leader; with JobObject it also makes the job kill-on-close.
    command.wrap(KillOnDrop);
    let mut child = command.spawn()?;
    let missing = || std::io::Error::other("child pipe missing after spawn");
    let pipes = Pipes {
        stdin: child.stdin().take().ok_or_else(missing)?,
        stdout: child.stdout().take().ok_or_else(missing)?,
        stderr: child.stderr().take().ok_or_else(missing)?,
    };
    let pid = child.id();
    ids.set_leader(pid);
    let watchdog = start_watchdog(pid, launch.watchdog_grace, ids);
    let (kill_tx, kill_rx) = mpsc::channel(4);
    let (exit_tx, exit_rx) = watch::channel(None);
    let owner = Owner {
        pid,
        ids: ids.clone(),
    };
    let task = tokio::spawn(own(TreeGuard(child), kill_rx, exit_tx, owner));
    Ok((
        ServerChild {
            pid,
            _watchdog: watchdog,
            kill_tx,
            exit_rx,
            task,
            ids: ids.clone(),
        },
        pipes,
    ))
}

/// The leader's id as the owner task clears it once the leader is reaped.
struct Owner {
    pid: Option<u32>,
    ids: LiveIds,
}

/// Owner loop: wait for the leader, kill on request, then kill and reap the rest of the tree.
async fn own(
    mut guard: TreeGuard,
    mut kill_rx: mpsc::Receiver<()>,
    exit_tx: watch::Sender<Option<ExitInfo>>,
    owner: Owner,
) {
    let mut kill_requests_open = true;
    let status = loop {
        // Wait on the leader only (`inner_mut`): the wrapper's own `wait` also waits for every
        // other process in the job, which would delay exit detection while a tool runs.
        tokio::select! {
            status = guard.0.inner_mut().wait() => break status,
            request = kill_rx.recv(), if kill_requests_open => {
                if request.is_none() {
                    kill_requests_open = false;
                }
                if let Err(error) = guard.0.start_kill() {
                    debug!(%error, "kill request: tree already gone");
                }
            }
        }
    };
    // Reaped: from here on the id may be reused, so the last-resort stop must not signal it.
    owner.ids.leader_gone(owner.pid);
    let info = match &status {
        Ok(status) => exit_info(*status),
        Err(error) => {
            warn!(%error, "waiting for the server process failed");
            ExitInfo::default()
        }
    };
    exit_tx.send_replace(Some(info));
    // The leader is gone; remove whatever it left in its group/job and reap it.
    let _ = guard.0.start_kill();
    if tokio::time::timeout(Duration::from_secs(5), guard.0.wait())
        .await
        .is_err()
    {
        warn!("server process tree not fully reaped within 5 s");
    }
}

fn exit_info(status: ExitStatus) -> ExitInfo {
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal = None;
    ExitInfo {
        code: status.code(),
        signal,
    }
}

impl ServerChild {
    /// OS process id of the leader (bwrap when a launcher prefix is used).
    pub(crate) fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// A watch that becomes `Some` when the leader has exited.
    pub(crate) fn exit_watch(&self) -> watch::Receiver<Option<ExitInfo>> {
        self.exit_rx.clone()
    }

    /// Asks the owner to kill the whole tree (idempotent, non-blocking).
    pub(crate) fn kill(&self) {
        let _ = self.kill_tx.try_send(());
    }

    /// Waits up to `limit` for the leader to exit.
    pub(crate) async fn wait_exit(&self, limit: Duration) -> Option<ExitInfo> {
        let mut rx = self.exit_rx.clone();
        match tokio::time::timeout(limit, rx.wait_for(Option::is_some)).await {
            Ok(Ok(info)) => *info,
            _ => None,
        }
    }

    /// Kills the tree, waits for the exit and for the owner task to finish reaping (then the
    /// handle's `Drop` aborts nothing: the task is done).
    pub(crate) async fn kill_and_reap(mut self, limit: Duration) -> Option<ExitInfo> {
        self.kill();
        let exit = self.wait_exit(limit).await;
        if tokio::time::timeout(limit + Duration::from_secs(5), &mut self.task)
            .await
            .is_err()
        {
            warn!("server owner task did not finish reaping in time; aborting it");
        }
        exit
    }
}
