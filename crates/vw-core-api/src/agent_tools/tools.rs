//! The session tools behind a session credential: `processes_list`, `port_lease`, `process_start`,
//! `process_stop`, `process_restart`; the preview tools, `handbook_check`, the
//! browser tools and the on-demand tool switches (`on_demand`) are dispatched to their own modules. Every one acts on the credential's session
//! only; answers are small JSON objects, refusals plain text naming the owner.
//!
//! Depends on: vw-procmon, vw-agent-tools (`output`), `start`, `session`, `launches`, `core::Shared`,
//! `processes`. Called by: `server`.
//! Invariants:
//! - Stops go through the monitor's safe stop with `Authority::Session` (fact-owned processes only).
//! - The started command is shown to this session's own agent only (it wrote it); never to the Debug API.
//! - Blocking work (scans, stops, the start wait) runs on `spawn_blocking`.

use std::sync::Arc;

use serde_json::{Value, json};
use vw_agent_tools::wire::tools as names;

use super::AgentTools;
use super::start::{StartRequest, process_start};
use crate::core::Shared;

/// Runs one tool for `session_id`.
pub(crate) async fn call(
    shared: &Arc<Shared>,
    tools: &Arc<AgentTools>,
    session_id: &str,
    tool: &str,
    mut args: Value,
) -> Result<Value, String> {
    vw_agent_tools::video::check_tool(tool)?;
    let prepared = crate::ask::tool_calls::receipts::take(shared, session_id, tool, &mut args)?;
    let outcome = match prepared.execution {
        Some(execution) => execution.await,
        None => call_prepared(shared, tools, session_id, tool, args).await,
    };
    crate::approval_notice::result(outcome, prepared.approved)
}

/// Dispatch after the one-use hook receipt has been consumed.
pub(crate) async fn call_prepared(
    shared: &Arc<Shared>,
    tools: &Arc<AgentTools>,
    session_id: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    if tool == names::DOCUMENT_READ {
        return super::document_tools::read(shared, session_id, &args).await;
    }
    if names::PROTECTED_CHANGE_TOOLS.contains(&tool) {
        return crate::protected_changes::tool(shared, session_id, tool, args).await;
    }
    if let Some(answer) = small_tool(shared, tools, session_id, tool, &args).await {
        return answer;
    }
    crate::processes::note_vibe_server(shared);
    // Our own command tools skip Vibe's shell-command report, so they look for the gh shim's marker themselves.
    if matches!(
        tool,
        names::PROCESS_START | names::PROCESS_RESTART | names::SECRET_RUN
    ) {
        crate::github_reads::refresh_if_marked(shared).await;
    }
    if names::WORK_TOOLS.contains(&tool) {
        return super::work_tools::call(shared, session_id, tool, &args).await;
    }
    if names::HANDBOOK_TOOLS.contains(&tool) {
        return super::handbook_tool::call(shared, session_id, tool, &args).await;
    }
    if names::BROWSER_TOOLS.contains(&tool) {
        if shared.store.view(session_id).is_none() {
            return Err("this session is not open in Vibe Work right now".to_owned());
        }
        return super::browser_tools::call(shared, session_id, tool, args).await;
    }
    if names::SECRET_TOOLS.contains(&tool) {
        return super::secret_run::call(shared, tools, session_id, tool, args).await;
    }
    if names::PREVIEW_TOOLS.contains(&tool) {
        if shared.store.view(session_id).is_none() {
            return Err("this session is not open in Vibe Work right now".to_owned());
        }
        return super::preview_tools::call(shared, session_id, tool, &args).await;
    }
    let is_start = matches!(tool, names::PROCESS_START | names::PROCESS_RESTART);
    let start_wait = tools.config.start_wait;
    let restarted_from = (tool == names::PROCESS_RESTART)
        .then(|| args.get("pid").cloned())
        .flatten();
    let pending = json!({
        "status": "start/restart pending",
        "shell": null,
        "note": "the request is still in progress and may have started the process; call processes_list before repeating it",
    });
    let (spawned, received) = tokio::sync::oneshot::channel();
    let (shared, tools) = (shared.clone(), tools.clone());
    let (id, tool) = (session_id.to_owned(), tool.to_owned());
    let job = tokio::task::spawn_blocking(move || {
        if shared.store.view(&id).is_none() {
            return Err("this session is not open in Vibe Work right now".to_owned());
        }
        match tool.as_str() {
            names::PROCESSES_LIST => processes_list(&shared, &tools, &id),
            names::PORT_LEASE => shared
                .procmon
                .lease(&id)
                .map(|ports| json!({"leasedPorts": ports}))
                .map_err(|e| format!("no ports could be leased: {e}")),
            names::PROCESS_START => {
                let command = args.get("command").and_then(Value::as_str).unwrap_or("");
                let port = match args.get("port") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(
                        v.as_u64()
                            .and_then(|p| u16::try_from(p).ok())
                            .ok_or("port must be a port number")?,
                    ),
                };
                process_start(
                    &shared,
                    &tools,
                    &StartRequest {
                        session_id: &id,
                        command,
                        cwd: args.get("cwd").and_then(Value::as_str),
                        port,
                    },
                    Some(spawned),
                    None,
                )
            }
            names::PROCESS_STOP => process_stop(&shared, &tools, &id, &args),
            names::PROCESS_RESTART => process_restart(&shared, &tools, &id, &args, spawned),
            _ => Err("unknown tool".to_owned()),
        }
    });
    if is_start {
        let mut result = super::process_reply::finish(job, received, start_wait, pending).await?;
        if let Some(pid) = restarted_from {
            result["restartedFrom"] = pid;
        }
        Ok(result)
    } else {
        job.await.map_err(|e| format!("the tool failed: {e}"))?
    }
}

/// The tools answered in one call by their own modules; `None` for every other tool.
async fn small_tool(
    shared: &Arc<Shared>,
    tools: &Arc<AgentTools>,
    session_id: &str,
    tool: &str,
    args: &Value,
) -> Option<Result<Value, String>> {
    #[cfg(feature = "video")]
    if tool == "video_frames" {
        return Some(super::video_tools::call(shared, tools, session_id, args.clone()).await);
    }
    if names::is_chrome(tool) && !shared.chrome_control.config.available() {
        let browser =
            crate::chrome_control::agent_browser::selected(shared, session_id, tool, args);
        return Some(Err(browser.text(crate::chrome_control::setup::ABSENT)));
    }
    if tool == names::CHROME_TOOL_LIST {
        return Some(Ok(crate::chrome_control::gateway::list(shared)));
    }
    if names::CHROME_TOOLS.contains(&tool) {
        return Some(
            crate::chrome_control::gateway::call(shared, session_id, tool, args.clone()).await,
        );
    }

    Some(match tool {
        names::SESSION_STATUS
        | names::CONNECTORS_LIST
        | names::CONNECTOR_SUGGEST
        | names::ADDON_SUGGEST => super::session_facts::call(shared, session_id, tool, args).await,
        names::FEEDBACK_DRAFT => crate::feedback::offer(shared, session_id, args.clone()).await,
        names::SEARCH => super::search_tool::call(shared, session_id, args.clone()).await,
        names::READ_ALOUD => crate::read_aloud::tool(shared, session_id, args).await,
        names::PANEL_READ => crate::panel_read::tool(shared, session_id, args).await,
        names::USE_HEADLESS_BROWSER => {
            crate::on_demand::tool(shared, &tools.on_demand, session_id, tool)
        }
        _ => return None,
    })
}

/// The guards a start the core makes itself runs first (Vibe's bash hook never sees it): the kill guard,
/// the git guard and safety reminders. A restart re-checks its stored command.
pub(super) async fn guard_start(
    shared: &Arc<Shared>,
    session_id: &str,
    tool: &str,
    args: &Value,
) -> Result<(), String> {
    // The core starts these commands itself, so Vibe's bash hook never sees them: the same kill-guard
    // decision runs here first. A restart re-checks its stored command.
    let to_check = match tool {
        names::PROCESS_START | names::PROCESS_RESTART => args
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    };
    if let Some(command) = to_check {
        // Keep the existing guard policy: a start's command card can answer its guards;
        // restarts retain their separate Git/process guard checks.
        let card = tool == names::PROCESS_START
            && crate::one_ask::of(shared).is_some_and(|ledger| {
                ledger.card_said_yes(
                    session_id,
                    &format!("mcp_{}.{tool}", vw_agent_tools::wire::SERVER_NAME),
                    args,
                )
            });
        let asked = crate::one_ask::Asked::Card(card);
        let found = vw_agent_tools::killparse::scan(&command);
        super::killcheck::guard(shared, session_id, &command, &found, asked).await??;
        // The git guard too (a `git push` started as a "process" is still a push).
        let folder = args
            .get("cwd")
            .and_then(Value::as_str)
            .map(std::path::PathBuf::from)
            .or_else(|| {
                shared
                    .store
                    .view(session_id)
                    .and_then(|v| crate::changes_facts::facts(&v.state).cwd)
            })
            .unwrap_or_default();
        super::gitcheck::guard(shared, session_id, &folder, &command, asked).await?;
        // The safety reminders (Vibe's hook never sees a command the core starts itself).
        crate::safety::guard(shared, session_id, &command, Some(&folder))?;
    }
    Ok(())
}

#[path = "process_lifecycle.rs"]
mod process_lifecycle;
pub(crate) use process_lifecycle::restart_launch;
pub(super) use process_lifecycle::{forget_finished, handoff_finished};
use process_lifecycle::{process_restart, process_stop, processes_list};
