//! The processes panel: one session's processes, their latest output, and Stop. The window also
//! shows a "Left running" list; this module serves the session part.
//!
//! Three kinds. A server the person started in the session's Terminal tab (`terminal`: a listening process in a
//! terminal shell's tree, named by its executable, never its arguments; the output is in the Terminal tab; the
//! person's Stop here stops it). Vibe's own background processes (session state `backgroundProcesses`; output through
//! `session/backgroundProcess/output` with `fromEnd`; Stop through `session/backgroundProcess/stop`, which since Vibe
//! 2.26.1 also stops what they started) end with Vibe or the session's channel; the processes the agent
//! started through Vibe Work's `process_start` keep running until stopped (output from their log; Stop
//! through the monitor's proven stop).
//!
//! Depends on: the gate (Vibe calls), `agent_tools::panel`, `processes` (blocking, `stop_process_for_user`),
//! base64. Called by: `left_running` (`processes` command ops `session`, `output`, `stopSession`).
//! Invariants: an id names a process of THIS session (a `vibe:` id must be in the session's state, a `started:` id
//! among its live started processes, a `terminal:` id among the listening processes of its terminals) or the request
//! is `notFound`; nothing is stopped without that check.

use base64::Engine as _;
use serde_json::{Value, json};
use vw_procmon::{ProcessRef, StartToken};

use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::left_running_view::{ProcessesResult, SessionProcessView};
use crate::processes::blocking;

/// Bytes of a Vibe background process's output read for the panel.
const VIBE_TAIL: u64 = 16_000;

fn not_here() -> CoreError {
    CoreError::new(
        ErrorCode::NotFound,
        "that process is not one of this session's (it may have ended)",
    )
}

fn started_id(process: ProcessRef) -> String {
    format!("started:{}-{}", process.pid, process.start.0)
}

fn terminal_id(process: ProcessRef) -> String {
    format!("terminal:{}-{}", process.pid, process.start.0)
}

fn parse_started(id: &str) -> Option<ProcessRef> {
    parse_ref(id.strip_prefix("started:")?)
}

fn parse_ref(id: &str) -> Option<ProcessRef> {
    let (pid, start) = id.split_once('-')?;
    Some(ProcessRef {
        pid: pid.parse().ok()?,
        start: StartToken(start.parse().ok()?),
    })
}

impl Core {
    /// Vibe's background processes of the session, from the store's copy of its state.
    fn vibe_processes(&self, session_id: &str) -> Vec<SessionProcessView> {
        let state = self.shared.store.view(session_id).map(|v| v.state);
        let mut items: Vec<SessionProcessView> = state
            .as_ref()
            .and_then(|s| s.pointer("/backgroundProcesses"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let id = p.get("processId").and_then(Value::as_str)?;
                Some(SessionProcessView {
                    id: format!("vibe:{id}"),
                    kind: "vibe".to_owned(),
                    command: p.get("command").and_then(Value::as_str).map(str::to_owned),
                    status: p
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned(),
                    pid: None,
                    ports: Vec::new(),
                    earlier: false,
                })
            })
            .collect();
        for tracked in self.shared.vibe_processes.live(session_id) {
            let id = format!("vibe:{}", tracked.id);
            if let Some(item) = items.iter_mut().find(|item| item.id == id) {
                item.pid = Some(tracked.process.pid);
                item.status = "running".into();
            } else {
                items.push(SessionProcessView {
                    id,
                    kind: "vibe".into(),
                    command: Some(tracked.command),
                    status: "running".into(),
                    pid: Some(tracked.process.pid),
                    ports: Vec::new(),
                    earlier: false,
                });
            }
        }
        items
    }

    /// The agent's live `process_start` processes of the session.
    async fn started_processes(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionProcessView>, CoreError> {
        let Some(tools) = self.shared.agent_tools.clone() else {
            return Ok(Vec::new());
        };
        let shared = self.shared.clone();
        let id = session_id.to_owned();
        let found =
            blocking(move || crate::agent_tools::panel::started(&shared, &tools, &id)).await?;
        Ok(found
            .into_iter()
            .map(|s| SessionProcessView {
                id: started_id(s.process),
                kind: "started".to_owned(),
                command: s.command,
                status: "running".to_owned(),
                pid: Some(s.process.pid),
                ports: s.ports,
                earlier: s.earlier,
            })
            .collect())
    }

    /// The listening processes started in the session's Terminal tab: each listening process in a terminal shell's
    /// tree (the shell itself excluded), named by its executable.
    async fn terminal_processes(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionProcessView>, CoreError> {
        let monitor = self.shared.procmon.clone();
        let id = session_id.to_owned();
        blocking(move || {
            let Ok(snapshot) = monitor.scan() else {
                return Vec::new();
            };
            let shells: Vec<ProcessRef> = monitor
                .registry()
                .for_session(&id)
                .into_iter()
                .filter(|p| p.label == crate::terminal_procmon::LABEL)
                .map(|p| p.process)
                .filter(|shell| {
                    snapshot
                        .alive(shell.pid)
                        .is_some_and(|p| p.start == Some(shell.start))
                })
                .collect();
            let mut found: Vec<SessionProcessView> = Vec::new();
            for listener in &snapshot.listeners {
                let Some(info) = listener.pid.and_then(|pid| snapshot.alive(pid)) else {
                    continue;
                };
                let Some(process) = info.reference() else {
                    continue;
                };
                if !shells
                    .iter()
                    .any(|shell| shell.pid != info.pid && snapshot.descends_from(info.pid, *shell))
                {
                    continue;
                }
                let item_id = terminal_id(process);
                if let Some(item) = found.iter_mut().find(|i| i.id == item_id) {
                    if !item.ports.contains(&listener.port) {
                        item.ports.push(listener.port);
                        item.ports.sort_unstable();
                    }
                    continue;
                }
                found.push(SessionProcessView {
                    id: item_id,
                    kind: "terminal".to_owned(),
                    command: info.exe_name(),
                    status: "running".to_owned(),
                    pid: Some(info.pid),
                    ports: vec![listener.port],
                    earlier: false,
                });
            }
            found
        })
        .await
    }

    /// `processes session`.
    pub(crate) async fn session_panel(
        &self,
        session_id: &str,
    ) -> Result<ProcessesResult, CoreError> {
        let mut items = self.started_processes(session_id).await?;
        let started: Vec<Option<u32>> = items.iter().map(|i| i.pid).collect();
        // A process the agent started is listed once, as the agent's, also when a terminal is its ancestor.
        items.extend(
            self.terminal_processes(session_id)
                .await?
                .into_iter()
                .filter(|t| !started.contains(&t.pid)),
        );
        items.extend(self.vibe_processes(session_id));
        // Running first, then the rest in the order found.
        items.sort_by_key(|i| i.status != "running");
        Ok(ProcessesResult::Session { items })
    }

    /// `processes output`.
    pub(crate) async fn panel_output(
        &self,
        session_id: &str,
        id: &str,
    ) -> Result<ProcessesResult, CoreError> {
        if let Some(process_id) = id.strip_prefix("vibe:") {
            if !self.vibe_processes(session_id).iter().any(|p| p.id == id) {
                return Err(not_here());
            }
            let params = json!({"sessionId": session_id, "processId": process_id, "fromEnd": true,
                "cursor": 0, "waitMs": 0, "maxBytes": VIBE_TAIL});
            let reply = self
                .shared
                .gate
                .call("session/backgroundProcess/output", params, None)
                .await
                .map_err(|e| CoreError::from_gate("session/backgroundProcess/output", &e))?;
            let encoded = reply.result.get("outputBase64").and_then(Value::as_str);
            // Vibe runs a background process in a terminal; on Windows ConPTY opens with control sequences
            // (`ESC[1t ESC[c ESC[?1004h ESC[?9001h`). The panel shows plain text, so they are removed here.
            let text = encoded
                .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
                .map(|bytes| vw_terminal::strip_ansi(&String::from_utf8_lossy(&bytes)));
            return Ok(ProcessesResult::Output {
                id: id.to_owned(),
                available: text.is_some(),
                text: text.unwrap_or_default(),
            });
        }
        if id.starts_with("terminal:") {
            if !self
                .terminal_processes(session_id)
                .await?
                .iter()
                .any(|p| p.id == id)
            {
                return Err(not_here());
            }
            // Its output is in the Terminal tab.
            return Ok(ProcessesResult::Output {
                id: id.to_owned(),
                available: false,
                text: String::new(),
            });
        }
        let process = parse_started(id).ok_or_else(not_here)?;
        if !self
            .started_processes(session_id)
            .await?
            .iter()
            .any(|p| p.id == id)
        {
            return Err(not_here());
        }
        let Some(tools) = self.shared.agent_tools.clone() else {
            return Err(not_here());
        };
        let text = blocking(move || crate::agent_tools::panel::tail(&tools, process)).await?;
        Ok(ProcessesResult::Output {
            id: id.to_owned(),
            available: text.is_some(),
            text: text.unwrap_or_default(),
        })
    }

    /// `processes stopSession`.
    pub(crate) async fn panel_stop(
        &self,
        session_id: &str,
        id: &str,
    ) -> Result<ProcessesResult, CoreError> {
        if let Some(process_id) = id.strip_prefix("vibe:") {
            if !self.vibe_processes(session_id).iter().any(|p| p.id == id) {
                return Err(not_here());
            }
            let tracked = self
                .shared
                .vibe_processes
                .live(session_id)
                .into_iter()
                .find(|item| item.id == process_id);
            let stop = self.shared.gate.call(
                "session/backgroundProcess/stop",
                json!({"sessionId": session_id, "processId": process_id}),
                None,
            );
            if let Some(tracked) = tracked {
                // Vibe can stop the OS process yet never resolve its startup barrier.
                // Bound that wait; the monitor always checks the captured native identity.
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), stop).await;
                let outcome = self.stop_process(session_id, tracked.process).await?;
                if !outcome.gone {
                    return Err(CoreError::new(
                        ErrorCode::Internal,
                        "Background process is still running",
                    ));
                }
            } else {
                stop.await
                    .map_err(|e| CoreError::from_gate("session/backgroundProcess/stop", &e))?;
            }
            return Ok(ProcessesResult::StoppedSession { id: id.to_owned() });
        }
        let process = if let Some(rest) = id.strip_prefix("terminal:") {
            let process = parse_ref(rest).ok_or_else(not_here)?;
            if !self
                .terminal_processes(session_id)
                .await?
                .iter()
                .any(|p| p.id == id)
            {
                return Err(not_here());
            }
            process
        } else {
            let process = parse_started(id).ok_or_else(not_here)?;
            if !self
                .started_processes(session_id)
                .await?
                .iter()
                .any(|p| p.id == id)
            {
                return Err(not_here());
            }
            process
        };
        let outcome = self.stop_process_for_user(process).await?;
        if !outcome.gone {
            return Err(CoreError::new(
                ErrorCode::Internal,
                format!("still running after the stop: {:?}", outcome.remaining),
            ));
        }
        Ok(ProcessesResult::StoppedSession { id: id.to_owned() })
    }
}
