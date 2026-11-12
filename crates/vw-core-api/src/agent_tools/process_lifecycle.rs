//! Process observation, retained output, stop and restart for the credential's chat.
//! Called by the tool dispatcher and session cleanup; uses the existing monitor authority and launch records.
//! Ownership checks and output retention order remain in the same functions.
use crate::agent_tools::launches::Launch;
use crate::agent_tools::session::{alive, refusal_text, tree_listeners, url};
use crate::agent_tools::start::{LABEL, StartRequest, process_start};
use crate::agent_tools::{AgentTools, FinishedHere, StartedHere};
use crate::core::Shared;
use serde_json::{Value, json};
use vw_agent_tools::output;
use vw_procmon::{Attribution, ProcessClass, ProcessRef, StopError};

pub(super) fn pid_arg(args: &Value) -> Result<u32, String> {
    args.get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok())
        .filter(|p| *p > 1 && i32::try_from(*p).is_ok())
        .ok_or_else(|| "pid must be a process id from processes_list".to_owned())
}

/// The identity of `pid` as this session knows it (registered, re-attached or launched), else the OS's now.
pub(super) fn known_ref(
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

pub(super) fn processes_list(
    shared: &Shared,
    tools: &AgentTools,
    session_id: &str,
) -> Result<Value, String> {
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
pub(super) fn snapshot_adopted(
    tools: &AgentTools,
    session_id: &str,
    snapshot: &vw_procmon::Snapshot,
) {
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
pub(super) fn ended(snapshot: &vw_procmon::Snapshot, process: ProcessRef) -> bool {
    !alive(snapshot, process) && !vw_procmon::is_alive(process)
}

/// Snapshot this run's natural exits before launch cleanup removes their logs.
pub(super) fn snapshot_exited(tools: &AgentTools, session_id: &str) {
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
pub(super) fn finished_here(tools: &AgentTools, session_id: &str) -> Vec<Value> {
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
                "lastOutput": vw_terminal::strip_ansi(&s.last_output),
            })
        })
        .collect()
}

pub(super) fn retain_finished(
    tools: &AgentTools,
    process: ProcessRef,
    entry: &StartedHere,
    status: String,
) {
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

pub(super) fn retain_launch(tools: &AgentTools, launch: &Launch, status: String) {
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
pub(super) fn finish_launch(tools: &AgentTools, process: ProcessRef, status: &str) {
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

pub(super) fn remember_finished(
    finished: &mut std::collections::VecDeque<FinishedHere>,
    entry: FinishedHere,
) {
    // A scan can observe death before the reaper, or two scans can copy the same launch.
    // The first snapshot owns the output; later readers may see its already-deleted log.
    if finished.iter().any(|s| s.process == entry.process) {
        return;
    }
    finished.push_back(entry);
    trim_finished(finished);
}

pub(in crate::agent_tools) fn trim_finished(
    finished: &mut std::collections::VecDeque<FinishedHere>,
) {
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

pub(in crate::agent_tools) fn forget_finished(
    finished: &mut std::collections::VecDeque<FinishedHere>,
    session_id: &str,
) {
    finished.retain(|s| s.session_id != session_id);
}

pub(in crate::agent_tools) fn handoff_finished(
    finished: &mut std::collections::VecDeque<FinishedHere>,
    old: &str,
    new: &str,
) {
    for entry in finished.iter_mut().filter(|s| s.session_id == old) {
        new.clone_into(&mut entry.session_id);
    }
    trim_finished(finished);
}

pub(super) fn process_stop(
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

pub(super) fn stop(
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

pub(crate) fn restart_launch(
    shared: &Shared,
    tools: &AgentTools,
    session_id: &str,
    args: &Value,
) -> Result<Launch, String> {
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
    Ok(launch)
}

pub(super) fn process_restart(
    shared: &Shared,
    tools: &AgentTools,
    session_id: &str,
    args: &Value,
    spawned: tokio::sync::oneshot::Sender<Value>,
) -> Result<Value, String> {
    let pid = pid_arg(args)?;
    let target = known_ref(shared, tools, session_id, pid)
        .ok_or_else(|| format!("pid {pid} is not running"))?;
    let launch: Launch = serde_json::from_value(args["_vwRestart"].clone())
        .map_err(|_| "The restart's approved launch is unavailable.".to_owned())?;
    if launch.process != target || launch.session_id != session_id {
        return Err("The process changed since its restart was approved.".into());
    }
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
#[path = "process_lifecycle_finished_tests.rs"]
mod finished_tests;
