//! The processes panel: one session's processes, their latest output, and Stop. The window also
//! shows a "Left running" list; this module serves the session part.
//!
//! Two kinds, and why both: Vibe's own background processes (session state `backgroundProcesses`; output through
//! `session/backgroundProcess/output` with `fromEnd`; Stop through `session/backgroundProcess/stop`, which since Vibe
//! 2.26.1 also stops what they started) end with Vibe or the session's channel; the processes the agent
//! started through Vibe Work's `process_start` keep running until stopped (output from their log; Stop
//! through the monitor's proven stop).
//!
//! Depends on: the gate (Vibe calls), `agent_tools::panel`, `processes` (blocking, `stop_process_for_user`),
//! base64. Called by: `left_running` (`processes` command ops `session`, `output`, `stopSession`).
//! Invariants: an id names a process of THIS session (a `vibe:` id must be in the session's state, a `started:` id
//! among its live started processes) or the request is `notFound`; nothing is stopped without that check.

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

fn parse_started(id: &str) -> Option<ProcessRef> {
    let (pid, start) = id.strip_prefix("started:")?.split_once('-')?;
    Some(ProcessRef {
        pid: pid.parse().ok()?,
        start: StartToken(start.parse().ok()?),
    })
}

impl Core {
    /// Vibe's background processes of the session, from the store's copy of its state.
    fn vibe_processes(&self, session_id: &str) -> Vec<SessionProcessView> {
        let state = self.shared.store.view(session_id).map(|v| v.state);
        state
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
            .collect()
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

    /// `processes session`.
    pub(crate) async fn session_panel(
        &self,
        session_id: &str,
    ) -> Result<ProcessesResult, CoreError> {
        let mut items = self.started_processes(session_id).await?;
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
            let text = encoded
                .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            return Ok(ProcessesResult::Output {
                id: id.to_owned(),
                available: text.is_some(),
                text: text.unwrap_or_default(),
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
            self.shared
                .gate
                .call(
                    "session/backgroundProcess/stop",
                    json!({"sessionId": session_id, "processId": process_id}),
                    None,
                )
                .await
                .map_err(|e| CoreError::from_gate("session/backgroundProcess/stop", &e))?;
            return Ok(ProcessesResult::StoppedSession { id: id.to_owned() });
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
