//! Which ports a session's preview may open, who owns any other port (for refusals that name the owner), and the
//! busy-port explanations of the session's servers. Built from the process
//! monitor (attribution and leases) and the `process_start` records.
//!
//! Depends on: vw-procmon, vw-preview (`PortOwner`), vw-agent-tools (`output`), `agent_tools::session` (owner
//! texts, URLs), `processes` (contexts), `core::Shared`. Called by: `previews` (commands), `agent_tools`
//! (preview tools).
//! Invariants:
//! - Blocking (OS scans, log reads): called on `spawn_blocking`.
//! - A port counts as this session's when a listener on it is attributed to the session or the port is leased to
//!   it and nobody else listens there. For the agent and the Debug API (`strict`), only
//!   PROVEN ports count: a listener attributed as a fact (confidence) or a leased port; a listener only
//!   inferred from the folder is `NotProven`. The user in the app's own window may open inferred ones (the list
//!   shows the process and pid).
//! - Views carry the executable name and pid only (never arguments).

use std::collections::{BTreeMap, BTreeSet};

use vw_agent_tools::output;
use vw_preview::PortOwner;
use vw_procmon::{Attribution, Confidence, Owner, PortExplanation, PortOption, busy_port_line};

use crate::agent_tools::session::{owner_text, url};
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::preview_view::{BusyOption, BusyPortView, PreviewTarget};

/// Busy-port explanations returned per list.
const MAX_BUSY: usize = 4;

/// A session's ports as seen now.
pub(crate) struct SessionPorts {
    /// Listeners of the session, then free leased ports.
    pub(crate) targets: Vec<PreviewTarget>,
    /// The session's leased ports.
    pub(crate) leased: Vec<u16>,
    /// Owner of every listening port (any session or app).
    owners: BTreeMap<u16, PortOwner>,
    /// Every port a listener attributed to any session holds (what a Local page may reach), as the re-check's
    /// [`LivePorts::servers`].
    pub(crate) servers: Vec<u16>,
}

impl SessionPorts {
    /// Ports the preview may navigate to (`strict`: proven ones only).
    pub(crate) fn allowed(&self, strict: bool) -> Vec<u16> {
        let mut ports: Vec<u16> = self
            .targets
            .iter()
            .filter(|t| !strict || t.proven)
            .map(|t| t.port)
            .collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }
}

/// Scans and attributes, for `session_id` (its lease is topped up first, like `processes_list`).
pub(crate) fn session_ports(shared: &Shared, session_id: &str) -> Result<SessionPorts, String> {
    let _ = shared.procmon.lease(session_id);
    let snapshot = shared.procmon.scan().map_err(|e| e.to_string())?;
    let sessions = crate::processes::contexts(&shared.store);
    let inputs = shared.procmon.inputs(&sessions);
    let mut attribution = Attribution::new(&snapshot, &inputs);
    let mut owners: BTreeMap<u16, PortOwner> = BTreeMap::new();
    let mut proven: BTreeSet<u16> = BTreeSet::new();
    let mut targets: Vec<PreviewTarget> = Vec::new();
    let mut servers: BTreeSet<u16> = BTreeSet::new();
    let leased = shared.procmon.leases().of(session_id);
    for item in attribution.listeners() {
        let port = item.listener.port;
        if matches!(item.attributed.owner, Owner::Session { .. }) {
            servers.insert(port);
        }
        let owner = match &item.attributed.owner {
            Owner::Session { session_id: s } if s == session_id => PortOwner::ThisSession,
            Owner::Session { .. } => {
                PortOwner::OtherSession(owner_text(shared, &item.attributed.owner))
            }
            Owner::OtherApp { .. } => {
                PortOwner::OtherApp(owner_text(shared, &item.attributed.owner))
            }
        };
        if owner == PortOwner::ThisSession && item.attributed.confidence == Some(Confidence::Fact) {
            proven.insert(port);
        }
        if owner == PortOwner::ThisSession && !targets.iter().any(|t| t.port == port) {
            let pid = item.listener.pid;
            targets.push(PreviewTarget {
                port,
                url: url(item.listener.address, port),
                pid,
                process_name: pid
                    .and_then(|p| snapshot.processes.get(&p))
                    .and_then(vw_procmon::ProcessInfo::exe_name),
                listening: true,
                proven: false,
            });
        }
        // A port with listeners of several owners: anyone else's claim wins (refuse rather than guess).
        let keep_existing = owners
            .get(&port)
            .is_some_and(|o| *o != PortOwner::ThisSession);
        if !keep_existing {
            owners.insert(port, owner);
        }
    }
    targets.retain(|t| owners.get(&t.port) == Some(&PortOwner::ThisSession));
    for target in &mut targets {
        target.proven = proven.contains(&target.port) || leased.contains(&target.port);
    }
    for port in &leased {
        if !owners.contains_key(port) {
            targets.push(PreviewTarget {
                port: *port,
                url: format!("http://127.0.0.1:{port}/"),
                pid: None,
                process_name: None,
                listening: false,
                proven: true,
            });
        }
    }
    Ok(SessionPorts {
        targets,
        leased,
        owners,
        servers: servers.into_iter().collect(),
    })
}

/// What the previews' port re-check sees.
pub(crate) struct LivePorts {
    /// Per asked session: listeners attributed to it (any confidence) and its leased ports.
    pub(crate) sessions: std::collections::HashMap<String, Vec<u16>>,
    /// Attributed listeners only; a lease without a listener cannot keep an open page live.
    pub(crate) listeners: std::collections::HashMap<String, Vec<u16>>,
    /// Every port a listener attributed to any session holds (what a Local page may reach).
    pub(crate) servers: Vec<u16>,
}

/// The ports each of `sessions` holds now, and every session server's port (re-check). One scan; leases
/// are not topped up (a re-check never keeps a lease alive).
///
/// # Errors
/// The scan's error.
pub(crate) fn live_ports(shared: &Shared, sessions: &[String]) -> Result<LivePorts, String> {
    let snapshot = shared.procmon.scan().map_err(|e| e.to_string())?;
    let contexts = crate::processes::contexts(&shared.store);
    let inputs = shared.procmon.inputs(&contexts);
    let mut attribution = Attribution::new(&snapshot, &inputs);
    let mut live: std::collections::HashMap<String, BTreeSet<u16>> = sessions
        .iter()
        .map(|s| {
            let leased: BTreeSet<u16> = shared.procmon.leases().of(s).into_iter().collect();
            (s.clone(), leased)
        })
        .collect();
    let mut servers: BTreeSet<u16> = BTreeSet::new();
    let mut listeners: std::collections::HashMap<String, BTreeSet<u16>> =
        std::collections::HashMap::new();
    for item in attribution.listeners() {
        if let Owner::Session { session_id } = &item.attributed.owner {
            servers.insert(item.listener.port);
            if let Some(ports) = live.get_mut(session_id) {
                ports.insert(item.listener.port);
                listeners
                    .entry(session_id.clone())
                    .or_default()
                    .insert(item.listener.port);
            }
        }
    }
    Ok(LivePorts {
        sessions: live
            .into_iter()
            .map(|(s, ports)| (s, ports.into_iter().collect()))
            .collect(),
        listeners: listeners
            .into_iter()
            .map(|(s, ports)| (s, ports.into_iter().collect()))
            .collect(),
        servers: servers.into_iter().collect(),
    })
}

/// Who `port` belongs to for `session_id` (the policy's ownership question). `strict` (agent, Debug API): a port
/// of this session that is not proven is `NotProven`.
pub(crate) fn port_owner(
    shared: &Shared,
    ports: &SessionPorts,
    session_id: &str,
    port: u16,
    strict: bool,
) -> PortOwner {
    if let Some(owner) = ports.owners.get(&port) {
        let proven = ports.targets.iter().any(|t| t.port == port && t.proven);
        return if *owner == PortOwner::ThisSession && strict && !proven {
            PortOwner::NotProven
        } else {
            owner.clone()
        };
    }
    match shared.procmon.leases().holder(port) {
        Some(holder) if holder == session_id => PortOwner::ThisSession,
        Some(holder) => {
            PortOwner::OtherSession(crate::agent_tools::session::session_name(shared, &holder))
        }
        None => PortOwner::Nobody,
    }
}

/// A explanation in the view the UI and the agent read, for `asking`.
pub(crate) fn busy_view(
    shared: &Shared,
    asking: &str,
    explanation: &PortExplanation,
    line: Option<String>,
) -> BusyPortView {
    let holder_item = explanation.holders.first();
    let holder = match holder_item.map(|h| &h.owner) {
        Some(Owner::Session { session_id }) if session_id == asking => "this session".to_owned(),
        Some(owner) => owner_text(shared, owner),
        None if explanation.hidden_holder => "unknown (not visible to Vibe Work)".to_owned(),
        None => "nothing (the port is free now)".to_owned(),
    };
    let options = explanation
        .options
        .iter()
        .map(|o| match o {
            PortOption::UseNextLeasedPort { port } => BusyOption::UseNextLeasedPort { port: *port },
            PortOption::StopOwner { process, .. } => BusyOption::StopOwner { pid: process.pid },
            PortOption::OpenRunning { url } => BusyOption::OpenRunning { url: url.clone() },
            PortOption::Retry => BusyOption::Retry {},
        })
        .collect();
    BusyPortView {
        port: explanation.port,
        holder,
        holder_pid: holder_item.and_then(|h| h.process.as_ref().map(|p| p.pid)),
        line: line.map(|l| l.chars().take(300).collect()),
        options,
    }
}

/// "Address in use" lines in the output of this run's `process_start` processes of the session, explained.
pub(crate) fn busy_ports(shared: &Shared, session_id: &str) -> Vec<BusyPortView> {
    let Some(tools) = &shared.agent_tools else {
        return Vec::new();
    };
    let logs: Vec<std::path::PathBuf> = tools
        .started
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .filter(|s| s.session_id == session_id)
        .map(|s| s.log.clone())
        .collect();
    let sessions = crate::processes::contexts(&shared.store);
    let mut seen: Vec<u16> = Vec::new();
    let mut views = Vec::new();
    for log in logs {
        let Ok(text) = output::tail(&log, 16 * 1024) else {
            continue;
        };
        for line in text.lines().rev() {
            let Some(busy) = busy_port_line(line) else {
                continue;
            };
            let Some(port) = busy.port.filter(|p| !seen.contains(p)) else {
                continue;
            };
            seen.push(port);
            if let Ok(explanation) = shared
                .procmon
                .explain_port(Some(session_id), port, &sessions)
            {
                views.push(busy_view(
                    shared,
                    session_id,
                    &explanation,
                    Some(line.to_owned()),
                ));
            }
            if views.len() >= MAX_BUSY {
                return views;
            }
        }
    }
    views
}

impl crate::core::Core {
    /// Stops the holder of `port` the explanation offers to stop (one of our sessions' processes only).
    pub(crate) async fn stop_busy_owner(
        &self,
        session_id: &str,
        port: u16,
    ) -> Result<(), CoreError> {
        let explanation = self.explain_port(Some(session_id.to_owned()), port).await?;
        let target = explanation.options.iter().find_map(|o| match o {
            vw_procmon::PortOption::StopOwner { process, .. } => Some(*process),
            _ => None,
        });
        let Some(target) = target else {
            return Err(CoreError::new(
                ErrorCode::Denied,
                format!(
                    "nothing on port {port} can be stopped from Vibe Work (it is not one of its sessions' processes)"
                ),
            ));
        };
        self.stop_process_for_user(target).await.map(|_| ())
    }
}
