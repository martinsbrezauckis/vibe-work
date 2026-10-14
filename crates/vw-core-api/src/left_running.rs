//! The `processes` command: what deleted sessions left running, the question right after a
//! delete, and Stop.
//!
//! The agent's dev servers (`process_start`, class `agent`) are built to outlive their session's channel, the app's
//! quit and a crash; only the user stops them. When the session that started them is deleted, the window asks once,
//! in the native dialog, "Also stop its N running processes?"; whatever keeps running is listed in the status bar's
//! "Left running" list with Stop, until it ends or is stopped.
//!
//! Depends on: vw-procmon (`left_running`, `stop_left_running`), vw-gate (`ConfirmationRequest`, `dialog_value`),
//! `processes` (session contexts, blocking calls), `session_end` (titles), `left_running_view`. Called by:
//! `dispatch` (`Command::Processes`).
//! Invariants:
//! - The Debug API is never shown a dialog and never has processes stopped for it implicitly: `afterDelete` from
//!   the Debug API returns the list, and the client stops what it wants with `stop` (its explicit request is the
//!   user's action).
//! - `stop` accepts only an `id` of the list; vw-procmon refuses anything else before a signal.
//! - Views never carry arguments or folders (vw-procmon's view rule).

use std::collections::BTreeSet;

use serde_json::json;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};
use vw_procmon::{LedgerEntry, ProcessRef, StartToken, StopError};

use crate::command::{Origin, requester};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::left_running_view::{LeftRunningView, ProcessesRequest, ProcessesResult};
use crate::processes::{blocking, contexts};

/// The method name the native dialog shows for the after-delete question.
const STOP_METHOD: &str = "processes/stop";

fn id_of(process: ProcessRef) -> String {
    format!("{}-{}", process.pid, process.start.0)
}

/// Parses an `id` from the list back into the identity it names.
fn parse_id(id: &str) -> Option<ProcessRef> {
    let (pid, start) = id.split_once('-')?;
    Some(ProcessRef {
        pid: pid.parse().ok()?,
        start: StartToken(start.parse().ok()?),
    })
}

/// "node on port 5173" / "python3 on ports 8000, 8001" / "pid 4242", for the dialog.
fn describe(view: &LeftRunningView) -> String {
    let name = view
        .name
        .as_deref()
        .map_or_else(|| format!("pid {}", view.pid), vw_gate::dialog_quote);
    match view.ports.as_slice() {
        [] => name,
        [port] => format!("{name} on port {port}"),
        ports => format!(
            "{name} on ports {}",
            ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

impl Core {
    fn view_of(&self, entry: &LedgerEntry) -> LeftRunningView {
        let session_id = entry.session_id.clone().unwrap_or_default();
        LeftRunningView {
            id: id_of(entry.process),
            session_title: self.shared.ends.title(&session_id),
            session_id,
            pid: entry.process.pid,
            name: entry.name.clone(),
            ports: entry.ports.clone(),
        }
    }

    /// Every process left running by a deleted session, still alive (gone ones are dropped; so are the titles of
    /// sessions that have none left).
    async fn left_running_views(&self) -> Result<Vec<LeftRunningView>, CoreError> {
        let monitor = self.shared.procmon.clone();
        let entries = blocking(move || monitor.left_running()).await?;
        let live: BTreeSet<&str> = entries
            .iter()
            .filter_map(|e| e.session_id.as_deref())
            .collect();
        self.shared.ends.keep_titles(&live);
        Ok(entries.iter().map(|e| self.view_of(e)).collect())
    }

    /// Stops one listed process. `Ok(Err(reason))` when the monitor refused or failed (the caller decides how to
    /// report it).
    async fn stop_listed(&self, target: ProcessRef) -> Result<Result<bool, String>, CoreError> {
        let monitor = self.shared.procmon.clone();
        let sessions = contexts(&self.shared.store);
        let outcome = blocking(move || monitor.stop_left_running(target, &sessions)).await?;
        Ok(match outcome {
            Ok(done) if done.gone => Ok(done.escalated),
            Ok(done) => Err(format!(
                "still running after the stop: {:?}",
                done.remaining
            )),
            Err(StopError::Refused(refusal)) => Err(refusal.to_string()),
            Err(other) => Err(other.to_string()),
        })
    }

    /// Executes one `processes` request for `origin`.
    ///
    /// # Errors
    /// `notFound` for a `stop` of an id the list does not hold (or that cannot be read); `denied` when the monitor
    /// refuses the stop (the reason names the owner); `internal` when the monitor task fails.
    pub(crate) async fn processes(
        &self,
        origin: Origin,
        request: ProcessesRequest,
    ) -> Result<ProcessesResult, CoreError> {
        match request {
            ProcessesRequest::LeftRunning {} => Ok(ProcessesResult::LeftRunning {
                items: self.left_running_views().await?,
            }),
            ProcessesRequest::AfterDelete { session_id } => {
                self.after_delete(origin, &session_id).await
            }
            ProcessesRequest::Session { session_id } => self.session_panel(&session_id).await,
            ProcessesRequest::Output { session_id, id } => {
                self.panel_output(&session_id, &id).await
            }
            ProcessesRequest::StopSession { session_id, id } => {
                self.panel_stop(&session_id, &id).await
            }
            ProcessesRequest::Stop { id } => {
                let items = self.left_running_views().await?;
                let Some(item) = items.into_iter().find(|i| i.id == id) else {
                    return Err(CoreError::new(
                        ErrorCode::NotFound,
                        "that process is not on the list of processes left running (it may have ended)",
                    ));
                };
                let target = parse_id(&item.id).ok_or_else(|| {
                    CoreError::new(ErrorCode::Internal, "a listed process has no valid id")
                })?;
                match self.stop_listed(target).await? {
                    Ok(escalated) => Ok(ProcessesResult::Stopped { item, escalated }),
                    Err(reason) => Err(CoreError::new(ErrorCode::Denied, reason)),
                }
            }
        }
    }

    /// The question right after a delete (see `ProcessesRequest::AfterDelete`).
    async fn after_delete(
        &self,
        origin: Origin,
        session_id: &str,
    ) -> Result<ProcessesResult, CoreError> {
        let left: Vec<LeftRunningView> = self
            .left_running_views()
            .await?
            .into_iter()
            .filter(|i| i.session_id == session_id)
            .collect();
        if left.is_empty() || origin == Origin::DebugApi {
            return Ok(ProcessesResult::AfterDelete {
                asked: false,
                stopped: Vec::new(),
                failed: Vec::new(),
                left,
            });
        }
        let n = left.len();
        let what: Vec<String> = left.iter().map(describe).collect();
        let request = ConfirmationRequest {
            method: STOP_METHOD.to_owned(),
            requester: requester(origin),
            summary: format!(
                "Also stop {n} running process{} from deleted session {}? Its agent started {}. Continue stops \
                 {}; Cancel keeps {} running, listed under \"Left running\" in the status bar.",
                if n == 1 { "" } else { "es" },
                vw_gate::dialog_quote(session_id),
                what.join(", "),
                if n == 1 { "it" } else { "them" },
                if n == 1 { "it" } else { "them" },
            ),
            params: json!({
                "processes": left
                    .iter()
                    .map(|i| json!({"name": i.name, "pid": i.pid, "ports": i.ports}))
                    .collect::<Vec<_>>(),
            }),
            always: None,
        };
        if self.shared.confirmer.confirm(&request).await != Confirmation::Approved {
            return Ok(ProcessesResult::AfterDelete {
                asked: true,
                stopped: Vec::new(),
                failed: Vec::new(),
                left,
            });
        }
        let mut stopped = Vec::new();
        let mut failed = Vec::new();
        let mut still = Vec::new();
        for item in left {
            let Some(target) = parse_id(&item.id) else {
                continue;
            };
            match self.stop_listed(target).await? {
                Ok(_) => stopped.push(item),
                Err(reason) => {
                    failed.push(format!("{}: {reason}", describe(&item)));
                    still.push(item);
                }
            }
        }
        Ok(ProcessesResult::AfterDelete {
            asked: true,
            stopped,
            failed,
            left: still,
        })
    }
}
