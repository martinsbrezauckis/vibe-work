//! Terminals in the core: the `terminal` command handler, the bridge from terminal
//! lifecycle events to core events, and the output stream the UI attaches to.
//!
//! Depends on: vw-terminal (`TerminalManager`), `core::Shared` (session folders, events),
//! `terminal_view`, `error`. Called by: `dispatch` (`Command::Terminal`), `core` (construction,
//! shutdown, attach/ack for the UI host), `sessions`/`dispatch` (close on delete and archive).
//! Invariants:
//! - A terminal's folder is the session's folder from the core's own record (store or listing), or for the
//!   New chat screen the folder `new_chat` checked (else the home folder); never a path from the client.
//! - Input from every origin is written directly (the Debug API acts as the user; its dialog
//!   is gone).
//! - Terminal input never appears in logs or refusal events (only its length).

use tokio::sync::broadcast;
use vw_terminal::{AttachInfo, OpenRequest, TerminalError, TerminalEvents, TerminalId, ViewerSink};

use crate::command::CommandResult;
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::terminal_view::{TerminalRequest, TerminalResult};

/// Default columns and rows of a terminal opened without a size (the view fits right after).
const DEFAULT_SIZE: (u16, u16) = (80, 24);
/// Default and largest `read` sizes.
const READ_DEFAULT: u64 = 64 * 1024;
const READ_MAX: u64 = 1024 * 1024;
/// Largest single input (a paste); bigger ones are refused, so a client sends them in parts.
const WRITE_MAX: usize = 1024 * 1024;

/// Publishes terminal lifecycle events as core events.
pub(crate) struct EventBridge {
    events: broadcast::Sender<CoreEvent>,
    shells: crate::terminal_procmon::ShellRegistrar,
}

impl EventBridge {
    /// A bridge sending to `events` and keeping `registry` (the process monitor's) in step with the shells.
    pub(crate) fn new(
        events: broadcast::Sender<CoreEvent>,
        registry: vw_procmon::OwnProcessRegistry,
    ) -> Self {
        Self {
            events,
            shells: crate::terminal_procmon::ShellRegistrar::new(registry),
        }
    }
}

/// The terminal manager of a core: `PORT` from the monitor's session lease (or the config's override), and
/// every lifecycle event both published to clients and applied to the monitor's registry.
pub(crate) fn manager(
    config: &crate::core::CoreConfig,
    procmon: &vw_procmon::ProcMon,
    events: &broadcast::Sender<CoreEvent>,
) -> vw_terminal::TerminalManager {
    let ports: std::sync::Arc<dyn vw_terminal::PortLeaseSource> =
        config.terminal_ports.clone().unwrap_or_else(|| {
            std::sync::Arc::new(crate::terminal_procmon::MonitorLeases(procmon.clone()))
        });
    vw_terminal::TerminalManager::new(
        config.terminals.clone(),
        ports,
        std::sync::Arc::new(EventBridge::new(events.clone(), procmon.registry().clone())),
    )
}

impl TerminalEvents for EventBridge {
    fn publish(&self, event: vw_terminal::TerminalEvent) {
        self.shells.apply(&event);
        let _ = self.events.send(CoreEvent::Terminal {
            event: event.into(),
        });
    }
}

/// Maps a terminal error to the core's stable codes.
pub(crate) fn core_error(error: &TerminalError) -> CoreError {
    let code = match error {
        TerminalError::NotFound(_) | TerminalError::MissingFolder(_) => ErrorCode::NotFound,
        TerminalError::TooMany { .. } | TerminalError::NotRunning(_) => ErrorCode::Denied,
        TerminalError::BadSize { .. } => ErrorCode::InvalidParams,
        TerminalError::InputBusy(_) | TerminalError::StopTimeout { .. } => ErrorCode::Timeout,
        TerminalError::ShuttingDown => ErrorCode::ServerUnavailable,
        TerminalError::Spawn { .. } | TerminalError::Io { .. } => ErrorCode::Internal,
    };
    CoreError::new(code, error.to_string())
}

/// Bytes of a `write` request.
fn input_bytes(data: String, binary: bool) -> Result<Vec<u8>, CoreError> {
    let bytes = if binary {
        data.chars()
            .map(|c| u8::try_from(u32::from(c)))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(|_| {
                CoreError::new(
                    ErrorCode::InvalidParams,
                    "binary terminal input must be characters U+0000..U+00FF",
                )
            })?
    } else {
        data.into_bytes()
    };
    if bytes.len() > WRITE_MAX {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            format!(
                "terminal input of {} bytes is too large (at most {WRITE_MAX}); send it in parts",
                bytes.len()
            ),
        ));
    }
    Ok(bytes)
}

impl Core {
    /// Executes one terminal request (the same for every origin).
    pub(crate) async fn terminal(
        &self,
        request: TerminalRequest,
    ) -> Result<CommandResult, CoreError> {
        let terminals = &self.shared.terminals;
        let result = match request {
            TerminalRequest::Open {
                session_id,
                cols,
                rows,
            } => {
                let draft = crate::new_chat::is_draft(&self.shared, &session_id);
                let cwd = if draft {
                    crate::new_chat::terminal_folder(&self.shared)?
                } else {
                    self.shared.names.cwd_of(&self.shared.store, &session_id).ok_or_else(|| {
                        CoreError::new(
                            ErrorCode::NotFound,
                            "The session's folder is not available. Open the session from the list and try again.",
                        )
                    })?.into()
                };
                let info = terminals
                    .open(OpenRequest {
                        session_id,
                        cwd,
                        cols: cols.unwrap_or(DEFAULT_SIZE.0),
                        rows: rows.unwrap_or(DEFAULT_SIZE.1),
                    })
                    .await
                    .map_err(|e| match e {
                        TerminalError::TooMany { max, .. } if draft => {
                            crate::new_chat::too_many(max)
                        }
                        e => core_error(&e),
                    })?;
                TerminalResult::Terminal {
                    terminal: info.into(),
                }
            }
            TerminalRequest::List { session_id } => TerminalResult::Terminals {
                terminals: terminals
                    .list(session_id.as_deref())
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            },
            TerminalRequest::Write {
                terminal_id,
                data,
                binary,
            } => {
                let id = TerminalId::new(terminal_id);
                let bytes = input_bytes(data, binary == Some(true))?;
                terminals.write(&id, &bytes).map_err(|e| core_error(&e))?;
                TerminalResult::Done {}
            }
            TerminalRequest::Resize {
                terminal_id,
                cols,
                rows,
            } => {
                terminals
                    .resize(&TerminalId::new(terminal_id), cols, rows)
                    .map_err(|e| core_error(&e))?;
                TerminalResult::Done {}
            }
            TerminalRequest::Read {
                terminal_id,
                from,
                max_bytes,
                plain,
            } => {
                let max = max_bytes.unwrap_or(READ_DEFAULT).clamp(1, READ_MAX);
                let read = terminals
                    .read(
                        &TerminalId::new(terminal_id),
                        from,
                        usize::try_from(max).unwrap_or(usize::MAX),
                        plain == Some(true),
                    )
                    .map_err(|e| core_error(&e))?;
                TerminalResult::Output {
                    output: read.into(),
                }
            }
            TerminalRequest::Restart { terminal_id } => TerminalResult::Terminal {
                terminal: terminals
                    .restart(&TerminalId::new(terminal_id))
                    .await
                    .map_err(|e| core_error(&e))?
                    .into(),
            },
            TerminalRequest::Close { terminal_id } => {
                terminals
                    .close(&TerminalId::new(terminal_id))
                    .await
                    .map_err(|e| core_error(&e))?;
                TerminalResult::Done {}
            }
        };
        Ok(CommandResult::Terminal { result })
    }

    /// Attaches the app window's terminal view to `terminal_id`'s output: the scrollback is
    /// replayed into `sink`, then live bytes follow with flow control ([`Core::terminal_ack`]).
    /// A second attach replaces the first. Not a `Command`: only the host's own UI stream uses it.
    ///
    /// # Errors
    /// `notFound` for an unknown terminal.
    pub fn terminal_attach(
        &self,
        terminal_id: &str,
        sink: ViewerSink,
    ) -> Result<AttachInfo, CoreError> {
        self.shared
            .terminals
            .attach(&TerminalId::new(terminal_id), sink)
            .map_err(|e| core_error(&e))
    }

    /// The view `viewer` has rendered `bytes` more bytes.
    pub fn terminal_ack(&self, terminal_id: &str, viewer: u64, bytes: u64) {
        self.shared
            .terminals
            .ack(&TerminalId::new(terminal_id), viewer, bytes);
    }

    /// Detaches the view (the terminal keeps running and buffering).
    pub fn terminal_detach(&self, terminal_id: &str, viewer: u64) {
        self.shared
            .terminals
            .detach(&TerminalId::new(terminal_id), viewer);
    }

    /// The terminal manager (tests, and the process monitor once is wired).
    #[must_use]
    pub fn terminals(&self) -> &vw_terminal::TerminalManager {
        &self.shared.terminals
    }
}

/// Closes a session's terminals (archive, delete); failures are logged by the manager.
pub(crate) async fn close_session(shared: &Shared, session_id: &str) {
    let _ = shared.terminals.close_session(session_id).await;
}
