//! Busy-port explanation: given a port, or an "address already in use" line, name who holds
//! it and the options: use the next leased port, stop the owner (only when it is one of our sessions'), or
//! open the running one. The UI presents it; this crate only supplies the core API.
//!
//! Depends on: `attribution`, `leases`, `text`, `view`, `types`. Called by: `monitor`, tests.
//! Invariant: "stop" is offered only for a holder attributed to a session; another app is never offered.

use serde::{Deserialize, Serialize};

use crate::attribution::{Attribution, Inputs, Owner};
use crate::leases::{PortLeases, bind_refused};
use crate::types::{ProcessRef, Snapshot};
use crate::view::ListenerView;

/// What can be done about a busy port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PortOption {
    /// Use this port from the asking session's lease instead (free right now).
    UseNextLeasedPort {
        /// The port.
        port: u16,
    },
    /// Stop the holder (one of our sessions' processes; the stop itself is still checked).
    #[serde(rename_all = "camelCase")]
    StopOwner {
        /// The holder's session.
        session_id: String,
        /// The holder.
        process: ProcessRef,
    },
    /// Open what already runs there (a session's server).
    OpenRunning {
        /// Loopback URL.
        url: String,
    },
    /// Nothing holds it now: try again.
    Retry,
}

/// The explanation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortExplanation {
    /// The port (`None` when an "in use" line named none).
    pub port: Option<u16>,
    /// The session asking.
    pub asking_session: Option<String>,
    /// Listeners on the port (any address) with their owners.
    pub holders: Vec<ListenerView>,
    /// True when no listener is visible but a bind still fails (a holder this user cannot see: another
    /// user's on macOS, a Windows host listener under WSL, an excluded range).
    pub hidden_holder: bool,
    /// The session the port is leased to, if any.
    pub leased_to: Option<String>,
    /// What can be done, best first.
    pub options: Vec<PortOption>,
}

/// Explains `port` for `asking` (a session id, or `None` for the user in general).
#[must_use]
pub fn explain(
    snapshot: &Snapshot,
    inputs: &Inputs,
    leases: &PortLeases,
    asking: Option<&str>,
    port: Option<u16>,
) -> PortExplanation {
    let mut attribution = Attribution::new(snapshot, inputs);
    let holders: Vec<ListenerView> = match port {
        Some(p) => attribution
            .listeners()
            .iter()
            .filter(|l| l.listener.port == p)
            .map(|l| ListenerView::new(l, snapshot))
            .collect(),
        None => Vec::new(),
    };
    let hidden_holder = holders.is_empty() && port.is_some_and(bind_refused);
    let busy_ports: Vec<u16> = snapshot.listeners.iter().map(|l| l.port).collect();
    let mut options = Vec::new();
    if let Some(session) = asking
        && let Some(next) = leases
            .of(session)
            .into_iter()
            .find(|p| Some(*p) != port && !busy_ports.contains(p) && !bind_refused(*p))
    {
        options.push(PortOption::UseNextLeasedPort { port: next });
    }
    let mut offered_open = false;
    for holder in &holders {
        if let (Owner::Session { session_id }, Some(process)) = (&holder.owner, &holder.process) {
            // The view hides the start token of an inferred owner; the stop option needs it.
            if let Some(start) = snapshot.processes.get(&process.pid).and_then(|p| p.start) {
                let stop = PortOption::StopOwner {
                    session_id: session_id.clone(),
                    process: ProcessRef {
                        pid: process.pid,
                        start,
                    },
                };
                if !options.contains(&stop) {
                    options.push(stop);
                }
            }
            if !offered_open {
                options.push(PortOption::OpenRunning {
                    url: format!("http://127.0.0.1:{}/", holder.port),
                });
                offered_open = true;
            }
        }
    }
    if holders.is_empty() && !hidden_holder && port.is_some() {
        options.push(PortOption::Retry);
    }
    PortExplanation {
        port,
        asking_session: asking.map(str::to_owned),
        holders,
        hidden_holder,
        leased_to: port.and_then(|p| leases.holder(p)),
        options,
    }
}
