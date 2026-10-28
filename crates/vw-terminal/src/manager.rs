//! `TerminalManager`: every terminal of every session, by id.
//!
//! Depends on: `terminal`, `env`, `shell`, `config`, `seams`, `types`, tokio (`spawn_blocking` for
//! the blocking start/stop work). Called by: vw-core-api (commands, attach for the UI stream,
//! session archive/delete, shutdown), tests.
//! Invariants:
//! - A session never has more than `max_per_session` terminals, also under concurrent opens (a
//!   slot is reserved under the registry lock before the shell starts).
//! - After [`TerminalManager::close_all`] nothing new opens.
//! - The registry lock is a std mutex held only for map operations, never across a blocking call
//!   or an await.
//! - Dropping the last manager handle drops every terminal, and each process destructor kills its
//!   tree (no waiting): nothing outlives the core.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tracing::{info, warn};

use crate::config::TerminalConfig;
use crate::env::terminal_env;
use crate::error::TerminalError;
use crate::seams::{OwnProcess, PortLeaseSource, TerminalEvents};
use crate::shell::default_shell;
use crate::terminal::{Launch, Terminal};
use crate::types::{
    AttachInfo, OpenRequest, OutputRead, TerminalEvent, TerminalId, TerminalInfo, ViewerSink,
};

#[derive(Default)]
struct Registry {
    terminals: BTreeMap<TerminalId, Arc<Terminal>>,
    /// Opens in progress per session (reserved slots).
    opening: HashMap<String, usize>,
}

struct Inner {
    config: Arc<TerminalConfig>,
    ports: Arc<dyn PortLeaseSource>,
    events: Arc<dyn TerminalEvents>,
    registry: Mutex<Registry>,
    next_id: AtomicU64,
    accepting: AtomicBool,
}

/// The terminal registry. Cheap to clone (an `Arc` inside).
#[derive(Clone)]
pub struct TerminalManager {
    inner: Arc<Inner>,
}

fn lock(inner: &Inner) -> MutexGuard<'_, Registry> {
    inner
        .registry
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Runs blocking terminal work off the async runtime.
async fn blocking<T: Send + 'static>(
    id: &TerminalId,
    work: impl FnOnce() -> Result<T, TerminalError> + Send + 'static,
) -> Result<T, TerminalError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| TerminalError::Io {
            id: id.clone(),
            reason: format!("terminal worker failed: {error}"),
        })?
}

impl TerminalManager {
    /// A manager with `config`, handing out `PORT` from `ports` and publishing to `events`.
    #[must_use]
    pub fn new(
        config: TerminalConfig,
        ports: Arc<dyn PortLeaseSource>,
        events: Arc<dyn TerminalEvents>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                config: Arc::new(config),
                ports,
                events,
                registry: Mutex::new(Registry::default()),
                next_id: AtomicU64::new(1),
                accepting: AtomicBool::new(true),
            }),
        }
    }

    /// The settings in use.
    #[must_use]
    pub fn config(&self) -> &TerminalConfig {
        &self.inner.config
    }

    fn get(&self, id: &TerminalId) -> Result<Arc<Terminal>, TerminalError> {
        lock(&self.inner)
            .terminals
            .get(id)
            .cloned()
            .ok_or_else(|| TerminalError::NotFound(id.clone()))
    }

    /// Opens a terminal for `request.session_id` in `request.cwd`.
    ///
    /// # Errors
    /// `ShuttingDown`, `TooMany` (cap reached), `MissingFolder`, `BadSize`, `Spawn`.
    pub async fn open(&self, request: OpenRequest) -> Result<TerminalInfo, TerminalError> {
        if !self.inner.accepting.load(Ordering::SeqCst) {
            return Err(TerminalError::ShuttingDown);
        }
        if !(2..=1000).contains(&request.cols) || !(1..=500).contains(&request.rows) {
            return Err(TerminalError::BadSize {
                cols: request.cols,
                rows: request.rows,
            });
        }
        if !request.cwd.is_dir() {
            return Err(TerminalError::MissingFolder(
                request.cwd.display().to_string(),
            ));
        }
        let max = self.inner.config.max_per_session;
        {
            let mut registry = lock(&self.inner);
            let open = registry
                .terminals
                .values()
                .filter(|t| t.session_id() == request.session_id)
                .count();
            let reserved = registry
                .opening
                .entry(request.session_id.clone())
                .or_default();
            if open + *reserved >= max {
                return Err(TerminalError::TooMany {
                    session_id: request.session_id,
                    max,
                });
            }
            *reserved += 1;
        }
        let id = TerminalId::new(format!(
            "term-{}",
            self.inner.next_id.fetch_add(1, Ordering::SeqCst)
        ));
        let opened = self.spawn(id.clone(), request.clone()).await;
        let inserted = {
            let mut registry = lock(&self.inner);
            if let Some(reserved) = registry.opening.get_mut(&request.session_id) {
                *reserved = reserved.saturating_sub(1);
                if *reserved == 0 {
                    registry.opening.remove(&request.session_id);
                }
            }
            match opened {
                Err(error) => return Err(error),
                // close_all ran while this one started: it must not survive the shutdown.
                Ok(terminal) if !self.inner.accepting.load(Ordering::SeqCst) => Err(terminal),
                Ok(terminal) => {
                    registry.terminals.insert(id.clone(), terminal.clone());
                    Ok(terminal)
                }
            }
        };
        let terminal = match inserted {
            Ok(terminal) => terminal,
            Err(late) => {
                let _ = blocking(&id, move || late.close()).await;
                return Err(TerminalError::ShuttingDown);
            }
        };
        let info = terminal.info();
        info!(terminal = %id, session = %info.session_id, pid = ?info.pid, "terminal opened");
        self.inner
            .events
            .publish(TerminalEvent::Opened(info.clone()));
        Ok(info)
    }

    async fn spawn(
        &self,
        id: TerminalId,
        request: OpenRequest,
    ) -> Result<Arc<Terminal>, TerminalError> {
        let config = self.inner.config.clone();
        let events = self.inner.events.clone();
        let ports = self.inner.ports.clone();
        let session_id = request.session_id.clone();
        let task_id = id.clone();
        blocking(&task_id, move || {
            // A lease may scan the OS tables, so it is asked here, off the async runtime.
            let port = ports.port_for(&request.session_id);
            let mut env = terminal_env(&config.parent_env.vars(), port);
            for (key, value) in &config.extra_env {
                env.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
                env.push((key.clone(), value.clone()));
            }
            vw_supervisor::isolate_credentials(&mut env);
            let shell = config.shell.clone().unwrap_or_else(|| default_shell(&env));
            let launch = Launch {
                id,
                session_id: request.session_id,
                cwd: request.cwd,
                shell,
                env,
                cols: request.cols,
                rows: request.rows,
            };
            Terminal::open(launch, config, events)
                .map_err(|reason| TerminalError::Spawn { session_id, reason })
        })
        .await
    }

    /// The terminals of `session_id` (all sessions when `None`), oldest first.
    #[must_use]
    pub fn list(&self, session_id: Option<&str>) -> Vec<TerminalInfo> {
        let terminals: Vec<Arc<Terminal>> = lock(&self.inner)
            .terminals
            .values()
            .filter(|t| session_id.is_none_or(|s| t.session_id() == s))
            .cloned()
            .collect();
        terminals.iter().map(|t| t.info()).collect()
    }

    /// One terminal.
    ///
    /// # Errors
    /// `NotFound`.
    pub fn info(&self, id: &TerminalId) -> Result<TerminalInfo, TerminalError> {
        Ok(self.get(id)?.info())
    }

    /// Queues `data` as input (never blocks).
    ///
    /// # Errors
    /// `NotFound`, `NotRunning` (exited), `InputBusy` (the program is not reading).
    pub fn write(&self, id: &TerminalId, data: &[u8]) -> Result<(), TerminalError> {
        self.get(id)?.write(data)
    }

    /// Resizes the terminal.
    ///
    /// # Errors
    /// `NotFound`, `BadSize`, `Io`.
    pub fn resize(&self, id: &TerminalId, cols: u16, rows: u16) -> Result<(), TerminalError> {
        self.get(id)?.resize(cols, rows)
    }

    /// Up to `max` bytes of output from offset `from` (`None`: the newest `max` bytes).
    ///
    /// # Errors
    /// `NotFound`.
    pub fn read(
        &self,
        id: &TerminalId,
        from: Option<u64>,
        max: usize,
        plain: bool,
    ) -> Result<OutputRead, TerminalError> {
        Ok(self.get(id)?.output.read(from, max, plain))
    }

    /// Attaches `sink` as the terminal's viewer (replacing any other): the scrollback is replayed
    /// into it, then live output follows with flow control ([`TerminalManager::ack`]).
    ///
    /// # Errors
    /// `NotFound`.
    pub fn attach(&self, id: &TerminalId, sink: ViewerSink) -> Result<AttachInfo, TerminalError> {
        Ok(self.get(id)?.output.attach(sink))
    }

    /// The viewer has processed `bytes` more bytes. Unknown terminals and old viewers are ignored.
    pub fn ack(&self, id: &TerminalId, viewer: u64, bytes: u64) {
        if let Ok(terminal) = self.get(id) {
            terminal.output.ack(viewer, bytes);
        }
    }

    /// Detaches the viewer (the terminal keeps running and buffering).
    pub fn detach(&self, id: &TerminalId, viewer: u64) {
        if let Ok(terminal) = self.get(id) {
            terminal.output.detach(viewer);
        }
    }

    /// Restarts the shell (same id and scrollback).
    ///
    /// # Errors
    /// `NotFound`, `StopTimeout`, `Spawn`.
    pub async fn restart(&self, id: &TerminalId) -> Result<TerminalInfo, TerminalError> {
        let terminal = self.get(id)?;
        blocking(id, move || terminal.restart()).await
    }

    /// Closes one terminal: its whole process tree is stopped, then it is removed.
    ///
    /// # Errors
    /// `NotFound`; `StopTimeout` when the tree did not end in time (the terminal is removed anyway
    /// and its destructor kills what is left).
    pub async fn close(&self, id: &TerminalId) -> Result<(), TerminalError> {
        let terminal = self.get(id)?;
        let closing = terminal.clone();
        let result = blocking(id, move || closing.close()).await;
        lock(&self.inner).terminals.remove(id);
        info!(terminal = %id, ok = result.is_ok(), "terminal closed");
        self.inner.events.publish(TerminalEvent::Closed {
            id: id.clone(),
            session_id: terminal.session_id(),
        });
        result
    }

    /// Moves every terminal of session `old` to `new` (Vibe handed the conversation to a new session id: `/clear`,
    /// a rewind into a new session, a compaction with a new id). The shells keep running; their info
    /// and events name `new` from now on. Returns the moved terminals.
    #[must_use = "the moved items tell what followed the hand-off"]
    pub fn hand_off(&self, old: &str, new: &str) -> Vec<TerminalInfo> {
        let moved: Vec<Arc<Terminal>> = lock(&self.inner)
            .terminals
            .values()
            .filter(|t| t.session_id() == old)
            .cloned()
            .collect();
        moved
            .iter()
            .map(|t| {
                t.set_session(new);
                t.info()
            })
            .collect()
    }

    /// Closes every terminal of `session_id` (session archived or deleted). Returns the errors of
    /// terminals that did not stop in time (they are removed anyway).
    pub async fn close_session(&self, session_id: &str) -> Vec<TerminalError> {
        let ids: Vec<TerminalId> = self
            .list(Some(session_id))
            .into_iter()
            .map(|t| t.id)
            .collect();
        self.close_many(ids).await
    }

    /// Closes every terminal and refuses new ones (app quit).
    pub async fn close_all(&self) -> Vec<TerminalError> {
        self.inner.accepting.store(false, Ordering::SeqCst);
        let ids: Vec<TerminalId> = lock(&self.inner).terminals.keys().cloned().collect();
        self.close_many(ids).await
    }

    async fn close_many(&self, ids: Vec<TerminalId>) -> Vec<TerminalError> {
        let mut tasks = Vec::with_capacity(ids.len());
        for id in ids {
            let manager = self.clone();
            tasks.push(tokio::spawn(async move { manager.close(&id).await }));
        }
        let mut errors = Vec::new();
        for task in tasks {
            match task.await {
                Ok(Ok(()) | Err(TerminalError::NotFound(_))) => {}
                Ok(Err(error)) => {
                    warn!(%error, "terminal did not close cleanly");
                    errors.push(error);
                }
                Err(error) => warn!(%error, "terminal close task failed"),
            }
        }
        errors
    }

    /// Our own running process trees (pid, Unix session or Job Object members) for diagnostics and tests.
    /// The core registers terminals with the process monitor from lifecycle events instead.
    #[must_use]
    pub fn own_processes(&self) -> Vec<OwnProcess> {
        let terminals: Vec<Arc<Terminal>> = lock(&self.inner).terminals.values().cloned().collect();
        terminals.iter().filter_map(|t| t.own_process()).collect()
    }

    /// Every live process of a terminal's tree right now (Unix: its session; Windows: its job).
    ///
    /// # Errors
    /// `NotFound`.
    pub fn members(&self, id: &TerminalId) -> Result<Vec<u32>, TerminalError> {
        Ok(self.get(id)?.members())
    }

    /// The session a terminal belongs to.
    ///
    /// # Errors
    /// `NotFound`.
    pub fn session_of(&self, id: &TerminalId) -> Result<String, TerminalError> {
        Ok(self.get(id)?.session_id())
    }
}
