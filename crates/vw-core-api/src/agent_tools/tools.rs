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
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vw_agent_tools::output;
use vw_agent_tools::wire::tools as names;
use vw_procmon::{Attribution, ProcessClass, ProcessRef, StopError, bind_refused};

use super::launches::Launch;
use super::session::{alive, refusal_text, tree_listeners, url};
use super::start::{LABEL, StartRequest, process_start};
use super::{AgentTools, FinishedHere, StartedHere};
use crate::core::Shared;

/// Runs one tool for `session_id`.
pub(crate) async fn call(
    shared: &Arc<Shared>,
    tools: &Arc<AgentTools>,
    session_id: &str,
    tool: &str,
    mut args: Value,
) -> Result<Value, String> {
    if let Some(execution) =
        crate::ask::tool_calls::receipts::take(shared, session_id, tool, &mut args)?
    {
        return execution.await;
    }
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

/// The tools answered in one call by their own modules (read aloud, the side panel, the on-demand switches and
/// `chrome_outcome`); `None` for every other tool.
async fn small_tool(
    shared: &Arc<Shared>,
    tools: &Arc<AgentTools>,
    session_id: &str,
    tool: &str,
    args: &Value,
) -> Option<Result<Value, String>> {
    if names::is_chrome(tool) && !shared.chrome_control.config.available() {
        return Some(Err(crate::chrome_control::setup::ABSENT.to_owned()));
    }
    if tool == names::CHROME_TOOL_LIST {
        return Some(Ok(crate::chrome_control::gateway::list(shared, session_id)));
    }
    if names::CHROME_TOOLS.contains(&tool) {
        return Some(
            crate::chrome_control::gateway::call(shared, session_id, tool, args.clone()).await,
        );
    }

    Some(match tool {
        names::READ_ALOUD => crate::read_aloud::tool(shared, session_id, args).await,
        names::PANEL_READ => crate::panel_read::tool(shared, session_id, args).await,
        names::USE_YOUR_CHROME | names::USE_HEADLESS_BROWSER => {
            crate::on_demand::tool(shared, &tools.on_demand, session_id, tool)
        }
        names::CHROME_OUTCOME => Ok(crate::chrome_control::outcome_tool(shared, session_id)),
        _ => return None,
    })
}

/// The guards a start the core makes itself runs first (Vibe's bash hook never sees it): the kill guard,
/// the git guard and the safety reminders. A restart re-checks its stored
/// command.
pub(super) async fn guard_start(
    shared: &Arc<Shared>,
    tools: &Arc<AgentTools>,
    session_id: &str,
    tool: &str,
    args: &Value,
) -> Result<(), String> {
    // The core starts these commands itself, so Vibe's bash hook never sees them: the same kill-guard
    // decision runs here first. A restart re-checks its stored command.
    let to_check = match tool {
        names::PROCESS_START => args
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_owned),
        names::PROCESS_RESTART => {
            let (s, t, id) = (shared.clone(), tools.clone(), session_id.to_owned());
            let pid = pid_arg(args).ok();
            tokio::task::spawn_blocking(move || {
                let target = known_ref(&s, &t, &id, pid?)?;
                t.launches
                    .of(target)
                    .filter(|l| l.session_id == id)
                    .map(|l| l.command)
            })
            .await
            .map_err(|e| format!("the tool failed: {e}"))?
        }
        _ => None,
    };
    if let Some(command) = to_check {
        // Vibe's card for this exact `process_start` call (it shows the command) came before this call;
        // the user's yes there answers the guards' "may this command run" question. A restart's card shows only a
        // pid, not the command, so it does not count.
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

fn pid_arg(args: &Value) -> Result<u32, String> {
    args.get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok())
        .filter(|p| *p > 1 && i32::try_from(*p).is_ok())
        .ok_or_else(|| "pid must be a process id from processes_list".to_owned())
}

/// The identity of `pid` as this session knows it (registered, re-attached or launched), else the OS's now.
fn known_ref(
    shared: &Shared,
    tools: &AgentTools,
    session_id: &str,
    pid: u32,
) -> Option<ProcessRef> {
    let registered = shared
        .procmon
        .registry()
        .for_session(session_id)
        .into_iter()
        .map(|p| p.process);
    let launched = tools
        .launches
        .all()
        .into_iter()
        .filter(|l| l.session_id == session_id)
        .map(|l| l.process);
    let adopted = shared
        .procmon
        .inputs(&[])
        .adopted
        .into_iter()
        .filter(|a| a.session_id == session_id)
        .map(|a| a.process);
    registered
        .chain(launched)
        .chain(adopted)
        .find(|r| r.pid == pid)
        .or_else(|| vw_procmon::identity(pid))
}

fn processes_list(shared: &Shared, tools: &AgentTools, session_id: &str) -> Result<Value, String> {
    // The lease is topped up here too, so the agent sees its ports before its first start:
    // an empty list made the model guess).
    let _ = shared.procmon.lease(session_id);
    let snapshot = shared.procmon.scan().map_err(|e| e.to_string())?;
    let sessions = crate::processes::contexts(&shared.store);
    let inputs = shared.procmon.inputs(&sessions);
    let mut ours: Vec<(ProcessRef, bool)> = shared
        .procmon
        .registry()
        .for_session(session_id)
        .into_iter()
        .filter(|p| p.label == LABEL)
        .map(|p| (p.process, false))
        .collect();
    for adopted in inputs.adopted.iter().filter(|a| a.session_id == session_id) {
        if !ours.iter().any(|(r, _)| *r == adopted.process) {
            ours.push((adopted.process, true));
        }
    }
    // Snapshot exited output before the launch cleanup removes its log.
    snapshot_exited(tools, session_id);
    let mut processes = Vec::new();
    let mut covered: Vec<ProcessRef> = Vec::new();
    for (process, earlier) in ours {
        let earlier = earlier
            || !tools
                .started
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_key(&process);
        let launch = tools.launches.of(process);
        if ended(&snapshot, process) {
            shared.procmon.registry().unregister_process(process);
            finish_launch(tools, process, "exited");
            continue;
        }
        covered.push(process);
        if let Some(launch) = &launch {
            let _ = output::enforce_cap(&launch.log, output::LOG_CAP);
        }
        let listening = tree_listeners(&snapshot, process);
        // The name of what listens (the server), else of the started process (often the shell).
        let named = snapshot
            .listeners
            .iter()
            .filter(|l| listening.iter().any(|(_, p)| *p == l.port))
            .find_map(|l| l.pid)
            .unwrap_or(process.pid);
        processes.push(json!({
            "pid": process.pid,
            "name": snapshot.processes.get(&named).and_then(vw_procmon::ProcessInfo::exe_name),
            "status": if earlier { "started earlier, still running" } else { "running" },
            "command": launch.as_ref().map(|l| l.command.clone()),
            "cwd": launch.as_ref().map(|l| l.cwd.display().to_string()),
            "port": launch.as_ref().and_then(|l| l.port),
            "shell": launch.as_ref().and_then(|l| l.shell.clone()),
            "listening": listening.iter().map(|(_, p)| *p).collect::<Vec<_>>(),
            "urls": listening.iter().map(|(a, p)| url(*a, *p)).collect::<Vec<_>>(),
        }));
    }
    snapshot_adopted(tools, session_id, &snapshot);
    processes.extend(finished_here(tools, session_id));
    let mut attribution = Attribution::new(&snapshot, &inputs);
    let other_listeners: Vec<Value> = attribution
        .listeners()
        .into_iter()
        .filter(|l| l.attributed.owner.session() == Some(session_id))
        .filter(|l| {
            !l.listener.pid.is_some_and(|pid| {
                covered
                    .iter()
                    .any(|r| r.pid == pid || snapshot.descends_from(pid, *r))
            })
        })
        .map(|l| {
            json!({
                "pid": l.listener.pid,
                "name": l.listener.pid.and_then(|p| snapshot.processes.get(&p)).and_then(vw_procmon::ProcessInfo::exe_name),
                "port": l.listener.port,
                "url": url(l.listener.address, l.listener.port),
                "startedBy": if l.attributed.provenance == Some(ProcessClass::Agent) { "the agent outside process_start" } else { "unknown (found in this session's folder)" },
            })
        })
        .collect();
    let background: Vec<Value> = sessions
        .iter()
        .filter(|s| s.session_id == session_id)
        .flat_map(|s| s.background.iter())
        .map(|b| json!({"processId": b.process_id, "status": b.status, "command": b.command}))
        .collect();
    Ok(json!({
        "processes": processes,
        "otherListeners": other_listeners,
        "vibeBackgroundProcesses": background,
        "leasedPorts": shared.procmon.leases().of(session_id),
    }))
}

/// A reattached launch can disappear without an in-memory start record or a registry row.
fn snapshot_adopted(tools: &AgentTools, session_id: &str, snapshot: &vw_procmon::Snapshot) {
    for launch in tools
        .launches
        .all()
        .into_iter()
        .filter(|l| l.session_id == session_id)
    {
        if ended(snapshot, launch.process) {
            finish_launch(tools, launch.process, "exited");
        }
    }
}

/// A launch may have been registered after the scan; confirm death before deleting its log.
fn ended(snapshot: &vw_procmon::Snapshot, process: ProcessRef) -> bool {
    !alive(snapshot, process) && !vw_procmon::is_alive(process)
}

/// Snapshot this run's natural exits before launch cleanup removes their logs.
fn snapshot_exited(tools: &AgentTools, session_id: &str) {
    let mut started = tools
        .started
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut exited: Vec<ProcessRef> = started
        .iter()
        .filter(|(_, s)| s.session_id == session_id && s.started.exited().is_some())
        .map(|(r, _)| *r)
        .collect();
    exited.sort_by_key(|process| {
        tools
            .launches
            .of(*process)
            .map_or(0, |launch| launch.started_ms)
    });
    for process in exited {
        if let Some(entry) = started.remove(&process) {
            let code = entry.started.exited().flatten();
            retain_finished(
                tools,
                process,
                &entry,
                code.map_or_else(
                    || "exited (signal)".to_owned(),
                    |c| format!("exited (code {c})"),
                ),
            );
            tools.launches.remove(process);
        }
    }
}

/// Finished `process_start` processes, newest 8 per session.
fn finished_here(tools: &AgentTools, session_id: &str) -> Vec<Value> {
    tools
        .finished
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .rev()
        .filter(|s| s.session_id == session_id)
        .take(8)
        .map(|s| {
            json!({
                "pid": s.process.pid,
                "status": s.status,
                "command": s.command,
                "shell": s.shell,
                "lastOutput": s.last_output,
            })
        })
        .collect()
}

fn retain_finished(tools: &AgentTools, process: ProcessRef, entry: &StartedHere, status: String) {
    let last_output = output::tail(&entry.log, 1024).unwrap_or_default();
    let mut finished = tools
        .finished
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    remember_finished(
        &mut finished,
        FinishedHere {
            session_id: entry.session_id.clone(),
            process,
            status,
            // The active launch keeps the full command; a finished snapshot has a fixed cap.
            command: entry.command.chars().take(16_384).collect(),
            shell: entry.started.shell.to_owned(),
            last_output,
        },
    );
}

fn retain_launch(tools: &AgentTools, launch: &Launch, status: String) {
    let mut finished = tools
        .finished
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    remember_finished(
        &mut finished,
        FinishedHere {
            session_id: launch.session_id.clone(),
            process: launch.process,
            status,
            command: launch.command.chars().take(16_384).collect(),
            shell: launch.shell.clone().unwrap_or_default(),
            last_output: output::tail(&launch.log, 1024).unwrap_or_default(),
        },
    );
}

/// Serialize output capture and log removal with this run's reaper snapshots.
fn finish_launch(tools: &AgentTools, process: ProcessRef, status: &str) {
    let mut started = tools
        .started
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = started.remove(&process) {
        retain_finished(tools, process, &entry, status.to_owned());
    } else if let Some(launch) = tools.launches.of(process) {
        retain_launch(tools, &launch, status.to_owned());
    }
    tools.launches.remove(process);
}

fn remember_finished(finished: &mut std::collections::VecDeque<FinishedHere>, entry: FinishedHere) {
    // A scan can observe death before the reaper, or two scans can copy the same launch.
    // The first snapshot owns the output; later readers may see its already-deleted log.
    if finished.iter().any(|s| s.process == entry.process) {
        return;
    }
    finished.push_back(entry);
    trim_finished(finished);
}

pub(super) fn trim_finished(finished: &mut std::collections::VecDeque<FinishedHere>) {
    // Walk backward so a hand-off that merges two sessions also keeps only their newest eight.
    let mut counts = std::collections::HashMap::<String, usize>::new();
    let mut retained = std::collections::VecDeque::new();
    while let Some(entry) = finished.pop_back() {
        let count = counts.entry(entry.session_id.clone()).or_default();
        if *count < 8 {
            retained.push_front(entry);
            *count += 1;
        }
    }
    *finished = retained;
    while finished.len() > 64 {
        finished.pop_front();
    }
}

pub(super) fn forget_finished(
    finished: &mut std::collections::VecDeque<FinishedHere>,
    session_id: &str,
) {
    finished.retain(|s| s.session_id != session_id);
}

pub(super) fn handoff_finished(
    finished: &mut std::collections::VecDeque<FinishedHere>,
    old: &str,
    new: &str,
) {
    for entry in finished.iter_mut().filter(|s| s.session_id == old) {
        new.clone_into(&mut entry.session_id);
    }
    trim_finished(finished);
}

fn process_stop(
    shared: &Shared,
    tools: &AgentTools,
    session_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let pid = pid_arg(args)?;
    let target = known_ref(shared, tools, session_id, pid)
        .ok_or_else(|| format!("pid {pid} is not running"))?;
    stop(shared, tools, session_id, target)
}

fn stop(
    shared: &Shared,
    tools: &AgentTools,
    session_id: &str,
    target: ProcessRef,
) -> Result<Value, String> {
    let sessions = crate::processes::contexts(&shared.store);
    match shared.procmon.stop(session_id, target, &sessions) {
        Ok(outcome) => {
            if outcome.gone {
                finish_launch(tools, target, "stopped");
            }
            Ok(json!({
                "pid": target.pid,
                "stopped": outcome.gone,
                "alsoStopped": outcome.members.iter().filter(|p| **p != target.pid).collect::<Vec<_>>(),
                "freedPorts": outcome.ports,
                "stillRunning": outcome.remaining,
            }))
        }
        Err(StopError::Refused(refusal)) => Err(crate::guard_refusals::record(
            shared,
            session_id,
            crate::GuardKind::Process,
            crate::guard_refusals::step(shared, session_id, None),
            &refusal_text(shared, &refusal),
        )),
        Err(other) => Err(format!("the stop failed: {other}")),
    }
}

fn process_restart(
    shared: &Shared,
    tools: &AgentTools,
    session_id: &str,
    args: &Value,
    spawned: tokio::sync::oneshot::Sender<Value>,
) -> Result<Value, String> {
    let pid = pid_arg(args)?;
    let target = known_ref(shared, tools, session_id, pid)
        .ok_or_else(|| format!("pid {pid} is not running"))?;
    let Some(launch) = tools
        .launches
        .of(target)
        .filter(|l| l.session_id == session_id)
    else {
        return Err(crate::guard_refusals::record(
            shared,
            session_id,
            crate::GuardKind::Process,
            crate::guard_refusals::step(shared, session_id, None),
            &format!(
                "pid {pid} was not started with process_start in this session; only those can be restarted \
             (process_stop stops it)"
            ),
        ));
    };
    let reservation = launch
        .port
        .map(|port| {
            shared
                .procmon
                .leases()
                .reserve_restart(session_id, port, target)
                .map_err(|e| e.to_string())
        })
        .transpose()?;
    let stopped = stop(shared, tools, session_id, target)?;
    if stopped["stopped"] != true {
        return Err(format!(
            "restart could not stop pid {pid}; processes still running: {}; call processes_list",
            stopped["stillRunning"]
        ));
    }
    if let Some(port) = launch.port {
        let deadline = Instant::now() + Duration::from_secs(5);
        while bind_refused(port) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let cwd = launch.cwd.to_string_lossy().into_owned();
    let mut started = process_start(
        shared,
        tools,
        &StartRequest {
            session_id,
            command: &launch.command,
            cwd: Some(&cwd),
            port: launch.port,
        },
        Some(spawned),
        reservation,
    )?;
    started["restartedFrom"] = json!(pid);
    Ok(started)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod finished_tests {
    use super::{FinishedHere, forget_finished, handoff_finished, remember_finished};
    use std::collections::VecDeque;
    use vw_procmon::{ProcessRef, StartToken};

    #[test]
    fn a_stale_scan_does_not_finish_a_new_live_launch() {
        let process = vw_procmon::identity(std::process::id()).unwrap();
        let stale = vw_procmon::Snapshot {
            listeners: Vec::new(),
            processes: std::collections::BTreeMap::new(),
            me: vw_procmon::SelfInfo {
                pid: process.pid,
                pgid: None,
            },
            took: std::time::Duration::ZERO,
        };
        assert!(!super::ended(&stale, process));
    }

    fn entry(session: &str, pid: u32) -> FinishedHere {
        FinishedHere {
            session_id: session.to_owned(),
            process: ProcessRef {
                pid,
                start: StartToken(u64::from(pid)),
            },
            status: "stopped".to_owned(),
            command: "server".to_owned(),
            shell: "test".to_owned(),
            last_output: format!("output-{pid}"),
        }
    }

    #[test]
    fn finished_output_keeps_newest_eight_per_session_and_separates_sessions() {
        let mut rows = VecDeque::new();
        remember_finished(&mut rows, entry("other", 99));
        for pid in 1..=10 {
            remember_finished(&mut rows, entry("mine", pid));
        }
        assert_eq!(rows.len(), 9);
        assert_eq!(rows.front().unwrap().process.pid, 99);
        assert_eq!(
            rows.iter()
                .filter(|s| s.session_id == "mine")
                .map(|s| s.process.pid)
                .collect::<Vec<_>>(),
            (3..=10).collect::<Vec<_>>()
        );
        remember_finished(&mut rows, entry("mine", 7));
        assert_eq!(rows.len(), 9);
        assert_eq!(rows.back().unwrap().last_output, "output-10");
        let mut repeated = entry("mine", 7);
        repeated.last_output.clear();
        remember_finished(&mut rows, repeated);
        assert_eq!(
            rows.iter()
                .find(|s| s.process.pid == 7)
                .unwrap()
                .last_output,
            "output-7"
        );
    }

    #[test]
    fn finished_output_has_a_global_cap_across_sessions() {
        let mut rows = VecDeque::new();
        for pid in 1..=65 {
            remember_finished(&mut rows, entry(&format!("session-{pid}"), pid));
        }
        assert_eq!(rows.len(), 64);
        assert_eq!(rows.front().unwrap().process.pid, 2);
        assert_eq!(rows.back().unwrap().process.pid, 65);
    }

    #[test]
    fn handoff_merges_and_caps_history_then_deletion_clears_only_its_session() {
        let mut rows = VecDeque::new();
        for pid in 1..=7 {
            remember_finished(&mut rows, entry("old", pid));
            remember_finished(&mut rows, entry("new", pid + 100));
        }
        remember_finished(&mut rows, entry("other", 999));
        handoff_finished(&mut rows, "old", "new");
        assert_eq!(rows.iter().filter(|s| s.session_id == "new").count(), 8);
        assert!(rows.iter().any(|s| s.session_id == "other"));
        forget_finished(&mut rows, "new");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows.front().unwrap().session_id, "other");
    }
}
