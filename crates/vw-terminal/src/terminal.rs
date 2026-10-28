//! One terminal: its shell process (one per generation; a restart starts a new one with the same id
//! and scrollback), the threads that serve it, and its stop/restart logic.
//!
//! Depends on: `pty` (spawn), `contain` (tree kill), `flow` (output), `config`, `seams`, `types`.
//! Called by: `manager`.
//! Threads per running shell (none of them on the async runtime):
//! - reader: blocking reads of up to 64 KiB, pushed into the output (which applies flow control);
//! - input: writes queued keystrokes to the PTY (a full PTY input buffer blocks this thread, never a
//!   runtime worker);
//! - waiter: waits for the shell **concurrently** with the reader (ShellX `6ec6e648`: waiting for
//!   reader EOF first left zombies and blocked `ConPTY` readers), then kills what is left of its tree,
//!   closes the PTY (`ConPTY` releases the reader only then), lets the reader drain, and reports.
//!
//! One output flush worker per terminal (in `flow`) also survives shell restarts.
//!
//! Invariants:
//! - The waiter holds only weak references, so dropping the terminal drops the process, and the
//!   process's destructor kills its tree.
//! - A shell that exits by itself takes its leftover jobs with it (they are orphans nobody manages;
//!   same rule as vw-supervisor for the server's tools).
//! - `stop`, `restart` and the exit report are serialized per terminal (`lifecycle`).

use std::ffi::OsString;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, channel, sync_channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{ChildKiller, MasterPty};
use tracing::{debug, warn};

use crate::config::TerminalConfig;
use crate::conpty;
use crate::contain::Containment;
use crate::error::TerminalError;
use crate::flow::Output;
use crate::pty::{self, Spawned};
use crate::seams::{OwnProcess, TerminalEvents};
use crate::shell::ShellCommand;
use crate::types::{ExitInfo, TerminalEvent, TerminalId, TerminalInfo, TerminalState};

/// Bytes per PTY read; the output side batches smaller reads for its attached viewer.
const READ_CHUNK: usize = 64 * 1024;
/// How long the waiter lets the reader drain after the shell exited.
const READER_DRAIN: Duration = Duration::from_secs(2);
/// What the scrollback shows between two generations.
const RESTART_MARKER: &[u8] = b"\r\n\x1b[2m[restarted]\x1b[0m\r\n";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The exit of one shell, set once by its waiter.
#[derive(Default)]
struct ExitSlot {
    value: Mutex<Option<ExitInfo>>,
    set: Condvar,
}

impl ExitSlot {
    fn get(&self) -> Option<ExitInfo> {
        lock(&self.value).clone()
    }

    fn put(&self, info: ExitInfo) {
        *lock(&self.value) = Some(info);
        self.set.notify_all();
    }

    /// Waits up to `limit` for the exit; true when it happened.
    fn wait(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let mut value = lock(&self.value);
        while value.is_none() {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            value = self
                .set
                .wait_timeout(value, deadline - now)
                .map_or_else(|poison| poison.into_inner().0, |(guard, _)| guard);
        }
        true
    }
}

/// One shell generation.
struct Process {
    pid: Option<u32>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    input: Mutex<Option<SyncSender<Vec<u8>>>>,
    killer: Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>,
    contain: Arc<Containment>,
    exit: Arc<ExitSlot>,
    /// Set by `stop`: the exit is expected and is not reported as an `Exited` event.
    stopping: AtomicBool,
}

impl Process {
    /// Closes the PTY side we hold (writer thread ends, `ConPTY` releases the reader).
    fn release(&self) {
        lock(&self.input).take();
        lock(&self.master).take();
        lock(&self.killer).take();
    }

    /// Ends the tree: hang-up, grace, kill; then waits for the shell's exit. True when it exited.
    fn stop(&self, grace: Duration, timeout: Duration) -> bool {
        self.stopping.store(true, Ordering::SeqCst);
        if self.exit.get().is_some() {
            return true;
        }
        let tree_gone = self.contain.terminate(grace, timeout);
        if let Some(killer) = lock(&self.killer).as_mut() {
            // The shell itself, in case it left its own session (or the job was gone).
            let _ = killer.kill();
        }
        let exited = self.exit.wait(timeout);
        if !tree_gone {
            warn!(pid = ?self.pid, "terminal tree still had members after the kill");
        }
        exited
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // The manager (or the app) went away without closing: no shell may outlive it.
        if self.exit.get().is_none() {
            self.contain.kill_now();
            if let Some(killer) = lock(&self.killer).as_mut() {
                let _ = killer.kill();
            }
        }
    }
}

/// Everything a terminal needs to (re)start its shell.
pub(crate) struct Launch {
    pub(crate) id: TerminalId,
    pub(crate) session_id: String,
    pub(crate) cwd: PathBuf,
    pub(crate) shell: ShellCommand,
    pub(crate) env: Vec<(OsString, OsString)>,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

/// One terminal.
pub(crate) struct Terminal {
    pub(crate) id: TerminalId,
    /// The owning session; replaced only by a session id hand-off (`TerminalManager::hand_off`).
    session_id: Mutex<String>,
    cwd: PathBuf,
    shell: ShellCommand,
    env: Vec<(OsString, OsString)>,
    started_at_ms: u64,
    pub(crate) output: Arc<Output>,
    process: Mutex<Option<Arc<Process>>>,
    size: Mutex<(u16, u16)>,
    generation: AtomicU32,
    lifecycle: Mutex<()>,
    config: Arc<TerminalConfig>,
    events: Arc<dyn TerminalEvents>,
}

impl Terminal {
    /// The session this terminal belongs to now.
    pub(crate) fn session_id(&self) -> String {
        lock(&self.session_id).clone()
    }

    /// Moves the terminal to session `new` (a hand-off): its shell, output and `PORT` stay as they are.
    pub(crate) fn set_session(&self, new: &str) {
        new.clone_into(&mut lock(&self.session_id));
    }

    /// Creates the terminal and starts its first shell.
    pub(crate) fn open(
        launch: Launch,
        config: Arc<TerminalConfig>,
        events: Arc<dyn TerminalEvents>,
    ) -> Result<Arc<Self>, String> {
        let terminal = Arc::new(Self {
            id: launch.id,
            session_id: Mutex::new(launch.session_id),
            cwd: launch.cwd,
            shell: launch.shell,
            env: launch.env,
            started_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            output: Arc::new(
                Output::new(
                    config.scrollback_bytes,
                    config.high_water,
                    config.ack_timeout,
                )
                .map_err(|error| format!("cannot start the terminal output worker: {error}"))?,
            ),
            process: Mutex::new(None),
            size: Mutex::new((launch.cols, launch.rows)),
            generation: AtomicU32::new(0),
            lifecycle: Mutex::new(()),
            config,
            events,
        });
        terminal.launch()?;
        Ok(terminal)
    }

    /// Starts a shell for the current generation.
    fn launch(self: &Arc<Self>) -> Result<(), String> {
        let (cols, rows) = *lock(&self.size);
        let Spawned {
            master,
            reader,
            writer,
            mut child,
        } = pty::spawn(&self.shell, &self.cwd, &self.env, cols, rows)?;
        let pid = child.process_id();
        #[cfg(unix)]
        let contained = Containment::new(pid);
        #[cfg(windows)]
        let contained = Containment::assign(child.as_ref());
        let contain = match contained {
            Ok(contain) => Arc::new(contain),
            Err(error) => {
                let _ = child.kill();
                return Err(format!("cannot contain the shell's processes: {error}"));
            }
        };
        let (input_tx, input_rx) = sync_channel(self.config.input_queue.max(1));
        // Windows: the reader answers the pseudo console's startup cursor query (`conpty.rs`).
        let answer = cfg!(windows).then(|| input_tx.clone());
        let process = Arc::new(Process {
            pid,
            master: Mutex::new(Some(master)),
            input: Mutex::new(Some(input_tx)),
            killer: Mutex::new(Some(child.clone_killer())),
            contain: contain.clone(),
            exit: Arc::new(ExitSlot::default()),
            stopping: AtomicBool::new(false),
        });
        let name = |what: &str| format!("vw-term-{what}-{}", self.id);
        let (done_tx, done_rx) = channel::<()>();
        let output = self.output.clone();
        let threads = thread::Builder::new()
            .name(name("in"))
            .spawn(move || input_loop(writer, &input_rx))
            .and_then(|_| {
                thread::Builder::new().name(name("out")).spawn(move || {
                    read_loop(reader, &output, answer.as_ref());
                    let _ = done_tx.send(());
                })
            });
        if let Err(error) = threads {
            contain.kill_now();
            let _ = child.kill();
            return Err(format!("cannot start terminal threads: {error}"));
        }
        let waiter = Waiter {
            process: Arc::downgrade(&process),
            terminal: Arc::downgrade(self),
            contain,
            exit: process.exit.clone(),
            done: done_rx,
            grace: self.config.hangup_grace,
            timeout: self.config.close_timeout,
        };
        let spawned = thread::Builder::new()
            .name(name("wait"))
            .spawn(move || waiter.run(child));
        if let Err(error) = spawned {
            // Dropping `process` below kills the tree (its destructor); nothing waits on it.
            return Err(format!("cannot start the terminal waiter: {error}"));
        }
        *lock(&self.process) = Some(process);
        Ok(())
    }

    fn current(&self) -> Option<Arc<Process>> {
        lock(&self.process).clone()
    }

    /// The terminal as clients see it.
    pub(crate) fn info(&self) -> TerminalInfo {
        let process = self.current();
        let (cols, rows) = *lock(&self.size);
        let state = match process.as_ref().and_then(|p| p.exit.get()) {
            Some(exit) => TerminalState::Exited(exit),
            None => TerminalState::Running,
        };
        TerminalInfo {
            id: self.id.clone(),
            session_id: self.session_id(),
            cwd: self.cwd.display().to_string(),
            shell: self.shell.program.display().to_string(),
            pid: process.and_then(|p| p.pid),
            state,
            cols,
            rows,
            generation: self.generation.load(Ordering::SeqCst),
            output_end: self.output.end(),
            started_at_ms: self.started_at_ms,
        }
    }

    /// The running tree for the process monitor, if the shell runs.
    pub(crate) fn own_process(&self) -> Option<OwnProcess> {
        let process = self.current()?;
        if process.exit.get().is_some() {
            return None;
        }
        Some(OwnProcess {
            session_id: self.session_id(),
            terminal_id: self.id.to_string(),
            pid: process.pid?,
            unix_session: process.contain.unix_session(),
            job_members: process.contain.job_members(),
        })
    }

    /// Every live process of the running tree (tests and diagnostics).
    pub(crate) fn members(&self) -> Vec<u32> {
        self.current()
            .map(|p| p.contain.members())
            .unwrap_or_default()
    }

    /// Queues input for the shell.
    pub(crate) fn write(&self, data: &[u8]) -> Result<(), TerminalError> {
        let not_running = || TerminalError::NotRunning(self.id.clone());
        let process = self.current().ok_or_else(not_running)?;
        if process.exit.get().is_some() {
            return Err(not_running());
        }
        let input = lock(&process.input);
        let sender = input.as_ref().ok_or_else(not_running)?;
        match sender.try_send(data.to_vec()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(TerminalError::InputBusy(self.id.clone())),
            Err(TrySendError::Disconnected(_)) => Err(not_running()),
        }
    }

    /// Resizes the PTY (the shell gets SIGWINCH / the `ConPTY` resize). An exited terminal keeps the
    /// size for its next start.
    pub(crate) fn resize(&self, cols: u16, rows: u16) -> Result<(), TerminalError> {
        if !(2..=1000).contains(&cols) || !(1..=500).contains(&rows) {
            return Err(TerminalError::BadSize { cols, rows });
        }
        *lock(&self.size) = (cols, rows);
        let Some(process) = self.current() else {
            return Ok(());
        };
        let master = lock(&process.master);
        match master.as_ref() {
            Some(master) => master
                .resize(pty::size(cols, rows))
                .map_err(|e| TerminalError::Io {
                    id: self.id.clone(),
                    reason: format!("resize failed: {e}"),
                }),
            None => Ok(()),
        }
    }

    /// Stops the shell and its tree (blocking). Used by close and restart.
    pub(crate) fn stop(&self) -> Result<(), TerminalError> {
        let _serial = lock(&self.lifecycle);
        self.stop_locked()
    }

    fn stop_locked(&self) -> Result<(), TerminalError> {
        let Some(process) = self.current() else {
            return Ok(());
        };
        if process.stop(self.config.hangup_grace, self.config.close_timeout) {
            Ok(())
        } else {
            Err(TerminalError::StopTimeout {
                id: self.id.clone(),
                seconds: self.config.close_timeout.as_secs(),
                detail: String::new(),
            })
        }
    }

    /// Closes for good: flow control off, tree stopped (blocking).
    pub(crate) fn close(&self) -> Result<(), TerminalError> {
        self.output.close();
        self.stop()
    }

    /// Stops the shell (if running) and starts a new one with the same id and scrollback.
    pub(crate) fn restart(self: &Arc<Self>) -> Result<TerminalInfo, TerminalError> {
        let _serial = lock(&self.lifecycle);
        self.stop_locked()?;
        self.output.note(RESTART_MARKER);
        if cfg!(windows) {
            // The new pseudo console is told its cursor starts at row 1, column 1 (`conpty.rs`).
            self.output
                .note(&conpty::restart_preamble(lock(&self.size).1));
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.launch().map_err(|reason| TerminalError::Spawn {
            session_id: self.session_id(),
            reason,
        })?;
        let info = self.info();
        self.events.publish(TerminalEvent::Restarted(info.clone()));
        Ok(info)
    }
}

/// Reads the shell's output until the PTY closes. With `answer` (Windows), the pseudo console's
/// startup cursor query is removed from the output and answered on the input queue (`conpty.rs`).
fn read_loop(
    mut reader: Box<dyn Read + Send>,
    output: &Output,
    answer: Option<&SyncSender<Vec<u8>>>,
) {
    let mut buf = vec![0_u8; READ_CHUNK];
    let mut startup = conpty::StartupQuery::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => match answer {
                Some(input) => {
                    let (pass, reply) = startup.filter(&buf[..n]);
                    if reply && input.try_send(conpty::REPLY.to_vec()).is_err() {
                        // The input queue cannot be full this early; if it ever is, keep the query
                        // in the output so the view's own terminal answers it when it attaches.
                        output.push(conpty::QUERY);
                    }
                    if !pass.is_empty() {
                        output.push(&pass);
                    }
                }
                None => output.push(&buf[..n]),
            },
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            // EIO is how a Unix PTY master reports that every slave handle closed.
            Err(error) => {
                debug!(%error, "terminal reader ended");
                break;
            }
        }
    }
}

/// Writes queued input until the queue's sender is dropped or the PTY fails.
fn input_loop(mut writer: Box<dyn Write + Send>, queue: &Receiver<Vec<u8>>) {
    while let Ok(chunk) = queue.recv() {
        if writer
            .write_all(&chunk)
            .and_then(|()| writer.flush())
            .is_err()
        {
            break;
        }
    }
}

/// The waiter thread's state.
struct Waiter {
    process: Weak<Process>,
    terminal: Weak<Terminal>,
    contain: Arc<Containment>,
    exit: Arc<ExitSlot>,
    done: Receiver<()>,
    grace: Duration,
    timeout: Duration,
}

impl Waiter {
    fn run(self, mut child: Box<dyn portable_pty::Child + Send + Sync>) {
        let info = match child.wait() {
            Ok(status) => ExitInfo {
                exit_code: Some(status.exit_code()),
                signal: status.signal().map(str::to_owned),
            },
            Err(error) => {
                warn!(%error, "waiting for the terminal shell failed");
                ExitInfo {
                    exit_code: None,
                    signal: None,
                }
            }
        };
        // Leftover jobs of a shell that is gone are orphans: end them, then close the PTY.
        self.contain.terminate(self.grace, self.timeout);
        let process = self.process.upgrade();
        if let Some(process) = &process {
            process.release();
        }
        let _ = self.done.recv_timeout(READER_DRAIN);
        self.exit.put(info);
        let expected = process
            .as_ref()
            .is_none_or(|p| p.stopping.load(Ordering::SeqCst));
        drop(process);
        if let Some(terminal) = self.terminal.upgrade().filter(|_| !expected) {
            terminal
                .events
                .publish(TerminalEvent::Exited(terminal.info()));
        }
    }
}
