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
use vw_platform::sync::LockRecover;

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
    /// Leased ports of live session commands for which no socket listener was detected yet.
    pub(crate) pending: Vec<u16>,
    /// Owner of every listening port (any session or app).
    owners: BTreeMap<u16, PortOwner>,
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
    let scan = next_scan();
    let snapshot = shared.procmon.scan().map_err(|e| e.to_string())?;
    let sessions = crate::processes::contexts(&shared.store);
    let inputs = shared.procmon.inputs(&sessions);
    let mut attribution = Attribution::new(&snapshot, &inputs);
    let mut owners: BTreeMap<u16, PortOwner> = BTreeMap::new();
    let mut proven: BTreeSet<u16> = BTreeSet::new();
    let mut targets: Vec<PreviewTarget> = Vec::new();
    let leased = shared.procmon.leases().of(session_id);
    let mut chat_servers = ChatServers::new();
    for item in attribution.listeners() {
        let port = item.listener.port;
        note_listener(&mut chat_servers, &item, &snapshot);
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
    publish_chat_servers(shared, scan, chat_servers);
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
        pending: crate::preview_readiness::pending_ports(shared, session_id, &leased, &snapshot),
        targets,
        leased,
        owners,
    })
}

/// Per port seen in one scan: true while every listener on it is a chat session's, and each listener's process
/// (`None`: not identified).
type ChatServers = BTreeMap<u16, (bool, Vec<Option<vw_procmon::ProcessRef>>)>;

/// Scans in the order they started (a slower, older scan never replaces a newer scan's chat servers).
static SCANS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_scan() -> u64 {
    SCANS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
}

fn note_listener(
    servers: &mut ChatServers,
    item: &vw_procmon::AttributedListener,
    snapshot: &vw_procmon::Snapshot,
) {
    let (chat, processes) = servers
        .entry(item.listener.port)
        .or_insert((true, Vec::new()));
    *chat &= matches!(item.attributed.owner, Owner::Session { .. });
    processes.push(
        item.listener
            .pid
            .and_then(|pid| snapshot.processes.get(&pid))
            .and_then(vw_procmon::ProcessInfo::reference),
    );
}

/// Gives the preview the chats' servers scan `scan` found: ports whose every listener is attributed to a chat session
/// (`PreviewService::set_chat_servers`; one listener of another owner on the port keeps it out), with the check that
/// each one's server still runs ([`server_alive`]). Their processes are watched for exit (`PreviewHub::exit_watch`),
/// so the App preview's rules drop a stopped server's port at once.
fn publish_chat_servers(shared: &Shared, scan: u64, servers: ChatServers) {
    if let Some(hub) = &shared.preview {
        let servers: BTreeMap<u16, Vec<Option<vw_procmon::ProcessRef>>> = servers
            .into_iter()
            .filter_map(|(port, (chat, processes))| chat.then_some((port, processes)))
            .collect();
        let ports = servers.keys().copied().collect();
        if let Some(watch) = &hub.exit_watch {
            let processes: Vec<vw_procmon::ProcessRef> =
                servers.values().flatten().flatten().copied().collect();
            watch.watch(&processes);
        }
        hub.service
            .set_chat_servers(scan, ports, server_alive(servers));
    }
}

/// How long a port's "not held" answer is reused: every proxied preview request asks about the chat servers, and
/// one read of the socket table can take a millisecond or more. A "held" answer is never reused, so a listener that
/// takes over a vacated port is never reached on an earlier answer.
const ABSENT_REUSE: std::time::Duration = std::time::Duration::from_millis(200);

/// True for a port while the processes that listened there in the scan still run and still hold every listener on
/// it ([`vw_procmon::port_held_by`]: one port's socket table and those processes, no new scan), read at each ask. A
/// server that exited or closed its socket takes its port out at once, before another program's listener on the
/// port is reached (not only at the next scan); it stays out for [`ABSENT_REUSE`] at least. A listener whose process
/// the scan could not identify keeps its port out.
fn server_alive(
    servers: BTreeMap<u16, Vec<Option<vw_procmon::ProcessRef>>>,
) -> vw_preview::ServerAlive {
    let absent: std::sync::Mutex<std::collections::HashMap<u16, std::time::Instant>> =
        std::sync::Mutex::default();
    std::sync::Arc::new(move |port| {
        let Some(processes) = servers.get(&port) else {
            return false;
        };
        let lock = || {
            absent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        };
        if lock()
            .get(&port)
            .is_some_and(|at| at.elapsed() < ABSENT_REUSE)
        {
            return false;
        }
        let held = processes
            .iter()
            .copied()
            .collect::<Option<Vec<_>>>()
            .is_some_and(|known| vw_procmon::port_held_by(port, &known));
        if held {
            lock().remove(&port);
        } else {
            lock().insert(port, std::time::Instant::now());
        }
        held
    })
}

/// What the previews' port re-check sees.
pub(crate) struct LivePorts {
    /// Per asked session: listeners attributed to it (any confidence) and its leased ports.
    pub(crate) sessions: std::collections::HashMap<String, Vec<u16>>,
    /// Attributed listeners only; a lease without a listener cannot keep an open page live.
    pub(crate) listeners: std::collections::HashMap<String, Vec<u16>>,
}

/// The ports each of `sessions` holds now (re-check), and every chat's servers for the preview
/// (`publish_chat_servers`). One scan; leases are not topped up (a re-check never keeps a lease alive).
///
/// # Errors
/// The scan's error.
pub(crate) fn live_ports(shared: &Shared, sessions: &[String]) -> Result<LivePorts, String> {
    let scan = next_scan();
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
    let mut listeners: std::collections::HashMap<String, BTreeSet<u16>> =
        std::collections::HashMap::new();
    let mut chat_servers = ChatServers::new();
    for item in attribution.listeners() {
        note_listener(&mut chat_servers, &item, &snapshot);
        if let Owner::Session { session_id } = &item.attributed.owner
            && let Some(ports) = live.get_mut(session_id)
        {
            ports.insert(item.listener.port);
            listeners
                .entry(session_id.clone())
                .or_default()
                .insert(item.listener.port);
        }
    }
    publish_chat_servers(shared, scan, chat_servers);
    Ok(LivePorts {
        sessions: live
            .into_iter()
            .map(|(s, ports)| (s, ports.into_iter().collect()))
            .collect(),
        listeners: listeners
            .into_iter()
            .map(|(s, ports)| (s, ports.into_iter().collect()))
            .collect(),
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
        .lock_or_recover()
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

/// Resolves exactly one explicit address or local port before ownership and network checks.
pub(crate) fn navigation_text(
    url: Option<String>,
    port: Option<u16>,
    path: Option<&str>,
) -> Result<String, CoreError> {
    match (url, port) {
        (Some(url), None) => Ok(url),
        (None, Some(port)) => vw_preview::loopback_url(port, path)
            .map_err(|e| CoreError::new(ErrorCode::InvalidParams, e.to_string())),
        _ => Err(CoreError::new(
            ErrorCode::InvalidParams,
            "give either a url or a port",
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};

    /// Set in the child copy of this test binary: that copy is the stand-in server (see [`stand_in_server`]).
    const CHILD: &str = "VW_PREVIEW_PORTS_TEST_CHILD";

    /// The child copy: listens on a free loopback port and prints it; on a line from stdin it closes that socket and
    /// keeps running, still holding a second listener (as a dev server keeps its other ports). Without the child
    /// variable this test does nothing.
    #[test]
    fn stand_in_server() {
        if std::env::var_os(CHILD).is_none() {
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let kept = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        println!("{}", listener.local_addr().unwrap().port());
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        drop(listener);
        println!("closed");
        let _kept = kept;
        std::thread::sleep(std::time::Duration::from_secs(30));
    }

    /// A stand-in server: this test binary itself, so the test needs no tool on PATH on any platform.
    struct Server {
        child: std::process::Child,
        port: u16,
        output: BufReader<std::process::ChildStdout>,
    }

    impl Server {
        fn start() -> Self {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "preview_ports::tests::stand_in_server",
                    "--test-threads=1",
                    "--nocapture",
                    "--quiet",
                ])
                .env(CHILD, "1")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let mut output = BufReader::new(child.stdout.take().unwrap());
            let port = Self::line(&mut output).parse().unwrap();
            Self {
                child,
                port,
                output,
            }
        }

        /// The next line the server printed that is not the test harness's own.
        fn line(output: &mut BufReader<std::process::ChildStdout>) -> String {
            loop {
                let mut line = String::new();
                assert!(output.read_line(&mut line).unwrap() > 0, "server ended");
                let line = line.trim();
                if line.parse::<u16>().is_ok() || line == "closed" {
                    return line.to_owned();
                }
            }
        }

        fn close_socket(&mut self) {
            let stdin = self.child.stdin.as_mut().unwrap();
            stdin.write_all(b"close\n").unwrap();
            stdin.flush().unwrap();
            assert_eq!(Self::line(&mut self.output), "closed");
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// The published check follows the listening process itself: once it exited, its port is out at once (no scan).
    #[test]
    fn a_servers_port_leaves_when_its_process_exits() {
        if std::env::var_os(CHILD).is_some() {
            return;
        }
        let mut server = Server::start();
        let process = vw_procmon::identity(server.child.id());
        assert!(process.is_some());
        let unknown = 40_002;
        let alive = super::server_alive(
            [(server.port, vec![process]), (unknown, vec![None])]
                .into_iter()
                .collect(),
        );
        assert!(alive(server.port));
        assert!(!alive(unknown), "a listener without a known process");
        assert!(!alive(40_003), "a port the scan did not see");
        server.child.kill().unwrap();
        server.child.wait().unwrap();
        std::thread::sleep(super::ABSENT_REUSE);
        assert!(!alive(server.port), "its server exited");
    }

    /// PREDICT (before the check asked who listens on the port now): a server that closed its listening socket but
    /// kept running left its port admitted, because only the process was checked; a program that then took the port
    /// received the App preview's requests until the next scan.
    #[test]
    fn a_port_leaves_when_its_server_closes_the_socket_and_keeps_running() {
        if std::env::var_os(CHILD).is_some() {
            return;
        }
        let mut server = Server::start();
        let process = vw_procmon::identity(server.child.id());
        assert!(process.is_some());
        let alive = super::server_alive([(server.port, vec![process])].into_iter().collect());
        assert!(alive(server.port), "its server listens");
        server.close_socket();
        std::thread::sleep(super::ABSENT_REUSE);
        assert!(
            vw_procmon::is_alive(process.unwrap()),
            "the server still runs"
        );
        assert!(!alive(server.port), "its server no longer listens there");
        // Another program (this test process) takes the port over.
        let other = std::net::TcpListener::bind(("127.0.0.1", server.port)).unwrap();
        std::thread::sleep(super::ABSENT_REUSE);
        assert!(
            !alive(server.port),
            "a listener of another process on the port is not the chat's server"
        );
        drop(other);
    }

    /// PREDICT (before a positive answer stopped being reused): right after a request found the chat's server on its
    /// port, the server closed its socket and another program bound the port; the next request inside the reuse
    /// window received the earlier "held" answer and the App preview reached the other program's listener.
    #[test]
    fn a_takeover_right_after_a_positive_answer_is_refused() {
        if std::env::var_os(CHILD).is_some() {
            return;
        }
        let mut server = Server::start();
        let process = vw_procmon::identity(server.child.id());
        let alive = super::server_alive([(server.port, vec![process])].into_iter().collect());
        assert!(alive(server.port), "its server listens");
        // Cost of one positive answer: each proxied App preview request asks for every chat server's port.
        let started = std::time::Instant::now();
        for _ in 0..200 {
            assert!(alive(server.port));
        }
        eprintln!(
            "port check: {:?} per positive answer",
            started.elapsed() / 200
        );
        server.close_socket();
        let other = std::net::TcpListener::bind(("127.0.0.1", server.port)).unwrap();
        assert!(
            !alive(server.port),
            "a listener of another process on the port, at once, is not the chat's server"
        );
        drop(other);
    }
}
