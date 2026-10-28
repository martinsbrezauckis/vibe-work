//! `process_start` (and the start half of `process_restart`): the core starts the agent's long-running command
//! for its session so it outlives Vibe and the app, registers it as an `agent`
//! process of the session (a fact for the monitor), records the launch, and waits briefly for a listener.
//!
//! Depends on: vw-agent-tools (`spawn`, `output`), vw-procmon, `launches`, `session`, `core::Shared`. Called by:
//! `tools`.
//! Invariants:
//! - Only inside the session's folders; only on a port leased to the session; `PORT` always set.
//! - The environment is the configured allowlist plus `PORT` (no key, no `VW_*`).
//! - Registered and saved in the launch file before acknowledging the spawn. Ledger/readiness scans may
//!   finish after the reply; `process_reply` never lets them hide a successful spawn behind a timeout.
//! - Blocking: called on `spawn_blocking`.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use vw_agent_tools::output;
use vw_agent_tools::spawn::{AgentCommand, start};
use vw_procmon::{OwnProcess, PortReservation, ProcessClass, ProcessGroup, ProcessRef, local_urls};

use super::launches::Launch;
use super::session::{folders, inside, tree_listeners, url};
use super::{AgentTools, StartedHere};
use crate::core::Shared;

/// The label of processes started through `process_start` (registry, Debug API views). Never the command.
pub(crate) const LABEL: &str = "process_start";
/// What to start.
pub(crate) struct StartRequest<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) command: &'a str,
    pub(crate) cwd: Option<&'a str>,
    pub(crate) port: Option<u16>,
}

fn random_name() -> String {
    vw_agent_tools::credential::new_credential().map_or_else(
        |_| format!("{}-{}", std::process::id(), now_ms()),
        |c| c[..16].to_owned(),
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn check_command(command: &str) -> Result<(), String> {
    if command.is_empty() || command.contains('\0') {
        return Err("the command must be nonempty text without NUL bytes".to_owned());
    }
    Ok(())
}

/// Live `process_start` processes of a session (this run's registrations and re-attached ones).
pub(crate) fn live_count(shared: &Shared, tools: &AgentTools, session_id: &str) -> usize {
    let monitor = &shared.procmon;
    let mut refs: Vec<ProcessRef> = monitor
        .registry()
        .for_session(session_id)
        .into_iter()
        .filter(|p| p.label == LABEL)
        .map(|p| p.process)
        .collect();
    refs.extend(
        tools
            .launches
            .all()
            .into_iter()
            .filter(|l| l.session_id == session_id)
            .map(|l| l.process),
    );
    refs.sort();
    refs.dedup();
    refs.into_iter()
        .filter(|r| vw_procmon::is_alive(*r))
        .count()
}

/// Reserves an available session port, or checks the original port retained through a restart.
fn pick_port(
    shared: &Shared,
    session_id: &str,
    asked: Option<u16>,
    reservation: Option<PortReservation>,
) -> Result<PortReservation, String> {
    if let Some(reservation) = reservation {
        let port = reservation.port();
        let snapshot = shared.procmon.scan().map_err(|e| e.to_string())?;
        if snapshot.listeners.iter().any(|l| l.port == port) || vw_procmon::bind_refused(port) {
            return Err(format!(
                "restart port {port} is not available to bind yet; the old process was stopped; call processes_list and start again on an available leased port"
            ));
        }
        return Ok(reservation);
    }
    // A partial pool can still start a process; no available slot is reported by reserve below.
    if let Err(error) = shared.procmon.lease(session_id)
        && shared.procmon.leases().of(session_id).is_empty()
    {
        return Err(format!("no ports could be leased: {error}"));
    }
    let snapshot = shared.procmon.scan().map_err(|e| e.to_string())?;
    shared
        .procmon
        .leases()
        .reserve(session_id, asked, &|port| {
            snapshot.listeners.iter().any(|l| l.port == port)
        })
        .map_err(|e| e.to_string())
}

/// Starts `request` and waits up to the configured time for a listener, a URL or an exit. Blocking.
pub(crate) fn process_start(
    shared: &Shared,
    tools: &AgentTools,
    request: &StartRequest<'_>,
    mut spawned: Option<tokio::sync::oneshot::Sender<Value>>,
    reservation: Option<PortReservation>,
) -> Result<Value, String> {
    let command = request.command.trim();
    check_command(command)?;
    let folders = folders(shared, request.session_id)?;
    let cwd =
        crate::guard_refusals::folder(shared, request.session_id, inside(&folders, request.cwd))?;
    super::session::check_config_command(shared, &cwd, command)?;
    if let Some(running) = already_running(
        shared,
        tools,
        request.session_id,
        command,
        &cwd,
        &mut spawned,
    ) {
        return Ok(running);
    }
    let max = tools.config.max_per_session;
    if live_count(shared, tools, request.session_id) >= max {
        return Err(format!(
            "this session already runs {max} processes started with process_start; stop one first \
             (processes_list, process_stop)"
        ));
    }
    let reservation = pick_port(shared, request.session_id, request.port, reservation)?;
    let port = reservation.port();
    let log = tools
        .config
        .data_dir
        .join("output")
        .join(format!("{}.log", random_name()));
    let mut env = tools.config.process_env.clone();
    env.retain(|(k, _)| !k.to_string_lossy().eq_ignore_ascii_case("PORT"));
    env.push(("PORT".into(), port.to_string().into()));
    let started = start(&AgentCommand {
        command,
        cwd: &cwd,
        env: &env,
        log: &log,
    })
    .map_err(|e| format!("the command could not be started: {e}"))?;
    let process = identity_of(started.pid)
        .ok_or_else(|| "the process ended before Vibe Work could record it".to_owned());
    let process = match process {
        Ok(p) => p,
        Err(e) => return Ok(exited_early(&started, &log, port, &e)),
    };
    let _ = shared.procmon.registry().register(OwnProcess {
        session_id: Some(request.session_id.to_owned()),
        process,
        group: started
            .group
            .map_or(ProcessGroup::None, |pgid| ProcessGroup::Unix { pgid }),
        class: ProcessClass::Agent,
        label: LABEL.to_owned(),
        keep_running: false,
    });
    tools.launches.insert(Launch {
        session_id: request.session_id.to_owned(),
        process,
        command: command.to_owned(),
        shell: Some(started.shell.to_owned()),
        cwd: cwd.clone(),
        port: Some(port),
        log: log.clone(),
        started_ms: now_ms(),
    });
    tools
        .started
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            process,
            StartedHere {
                session_id: request.session_id.to_owned(),
                started: started.clone(),
                log: log.clone(),
                command: command.to_owned(),
            },
        );
    // The guard survives any tool reply timeout because this worker continues. Only a true spawn
    // failure/early exit drops it; the recorded identity releases the port when the process exits.
    reservation.record(process);
    if let Some(spawned) = spawned {
        let _ = spawned.send(json!({
            "pid": process.pid,
            "port": port,
            "shell": started.shell,
            "listening": [],
            "url": null,
            "status": "started (readiness pending)",
            "note": "started, waiting for the port; call processes_list for readiness before starting another process; stop it with process_stop",
        }));
    }
    let sessions = crate::processes::contexts(&shared.store);
    if let Err(error) = shared.procmon.refresh_ledger(&sessions) {
        tracing::warn!(%error, "ledger refresh after process_start failed");
    }
    Ok(wait_for_it(shared, tools, process, &started, &log, port))
}

/// The same command already running for the session in the same folder (the model can start a
/// duplicate next to a re-attached server 1 time in 3 despite the skill). Reused instead of started again:
/// the answer says so, with the running process's pid, port and URL.
fn already_running(
    shared: &Shared,
    tools: &AgentTools,
    session_id: &str,
    command: &str,
    cwd: &std::path::Path,
    spawned: &mut Option<tokio::sync::oneshot::Sender<Value>>,
) -> Option<Value> {
    let launch = tools.launches.all().into_iter().find(|l| {
        l.session_id == session_id
            && l.command.trim() == command
            && l.cwd == cwd
            && vw_procmon::is_alive(l.process)
    })?;
    if let Some(spawned) = spawned.take() {
        let _ = spawned.send(json!({
            "pid": launch.process.pid,
            "port": launch.port,
            "shell": launch.shell,
            "status": "already running",
            "reused": true,
            "note": "this exact command already runs for this session; call processes_list for readiness; reuse it",
        }));
    }
    // A failed readiness scan must not turn a known live command into a second spawn.
    let listening = shared
        .procmon
        .scan()
        .map(|snapshot| tree_listeners(&snapshot, launch.process))
        .unwrap_or_default();
    let url_main = listening
        .iter()
        .find(|(_, p)| Some(*p) == launch.port)
        .or_else(|| listening.first())
        .map(|(a, p)| url(*a, *p));
    Some(json!({
        "pid": launch.process.pid,
        "port": launch.port,
        "shell": launch.shell,
        "listening": listening.iter().map(|(_, p)| *p).collect::<Vec<_>>(),
        "url": url_main,
        "status": "already running",
        "reused": true,
        "note": "this exact command already runs for this session in this folder: reuse it (process_restart restarts it)",
    }))
}

/// The pid's identity (start token), retried briefly while the OS publishes the new process.
fn identity_of(pid: u32) -> Option<ProcessRef> {
    for _ in 0..50 {
        if let Some(found) = vw_procmon::identity(pid) {
            return Some(found);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

fn exited_early(
    started: &vw_agent_tools::spawn::Started,
    log: &std::path::Path,
    port: u16,
    why: &str,
) -> Value {
    json!({
        "pid": started.pid,
        "port": port,
        "shell": started.shell,
        "status": format!("exited ({why})"),
        "firstOutput": output::head(log, 20, 4096).unwrap_or_default(),
    })
}

fn wait_for_it(
    shared: &Shared,
    tools: &AgentTools,
    process: ProcessRef,
    started: &vw_agent_tools::spawn::Started,
    log: &std::path::Path,
    port: u16,
) -> Value {
    let deadline = Instant::now() + tools.config.start_wait;
    let mut listeners = Vec::new();
    let mut urls: Vec<String>;
    loop {
        std::thread::sleep(Duration::from_millis(150));
        let exited = started.exited();
        if let Ok(snapshot) = shared.procmon.scan() {
            listeners = tree_listeners(&snapshot, process);
        }
        urls = output::tail(log, 16 * 1024)
            .map(|text| local_urls(&text).into_iter().map(|u| u.text).collect())
            .unwrap_or_default();
        if exited.is_some() || !listeners.is_empty() || Instant::now() >= deadline {
            break;
        }
    }
    if !listeners.is_empty() {
        // Record the ports now (the first ledger write happened before it listened): a crash right after
        // still gives the re-attached process its port back in the session's lease.
        let sessions = crate::processes::contexts(&shared.store);
        if let Err(error) = shared.procmon.refresh_ledger(&sessions) {
            tracing::warn!(%error, "ledger refresh after the listener appeared failed");
        }
    }
    let status = match started.exited() {
        Some(Some(code)) => format!("exited (code {code})"),
        Some(None) => "exited (signal)".to_owned(),
        None if listeners.is_empty() => "running (not listening yet)".to_owned(),
        None => "running".to_owned(),
    };
    let ports: Vec<u16> = listeners.iter().map(|(_, p)| *p).collect();
    let url_main = listeners
        .iter()
        .find(|(_, p)| *p == port)
        .or_else(|| listeners.first())
        .map(|(a, p)| url(*a, *p));
    json!({
        "pid": process.pid,
        "port": port,
        "shell": started.shell,
        "listening": ports,
        "url": url_main,
        "urlsInOutput": urls,
        "status": status,
        "firstOutput": output::head(log, 20, 4096).unwrap_or_default(),
        "note": "keeps running after this turn and after Vibe Work closes; stop it with process_stop",
    })
}
