//! Port leases: each session gets a small set of free ports from an app-owned range, never two
//! sessions the same port, checked free at hand-out. A failed/exited start returns its port; session end
//! returns unused slots while pending and live agent processes retain their assignments.
//!
//! Depends on: `error`, std (bind test). Called by: `monitor` (which supplies the socket-table check),
//! tests.
//! Invariants:
//! - Every lease operation runs under one mutex (no await inside), so two threads can never hand out one
//!   port twice.
//! - A port is handed out only when the caller's busy check (socket table: no listener on any address) says
//!   free AND a bind test on `127.0.0.1:<port>` succeeds. The table is needed because macOS and Windows let
//!   a specific-address bind coexist with another process's wildcard listener; the bind test catches what
//!   the table cannot see (Windows excluded ranges, WSL-host listeners).
//! - In memory only: after an app restart the table is empty and sessions lease again; ports still held by
//!   re-attached processes are adopted back by their session (`adopt`).
//! - The cursor moves on after each hand-out, so a returned port is not handed out again at once (sockets
//!   in `TIME_WAIT`, a slow process exit).

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::error::LeaseError;
use crate::types::ProcessRef;

#[derive(Clone, Copy)]
struct Claim {
    id: u64,
    process: Option<ProcessRef>,
}

/// Default lease range. Below every default ephemeral range (Linux 32768-60999, Windows/macOS 49152+), away
/// from common dev-server ports and below the Kubernetes `NodePort` range (30000+). See the note.
pub const DEFAULT_RANGE: RangeInclusive<u16> = 28100..=28999;

/// Default ports per session.
pub const DEFAULT_PER_SESSION: usize = 4;

/// Lease settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseConfig {
    /// First port of the app-owned range.
    pub first: u16,
    /// Last port of the range (inclusive).
    pub last: u16,
    /// Ports per session.
    pub per_session: usize,
}

impl Default for LeaseConfig {
    fn default() -> Self {
        Self {
            first: *DEFAULT_RANGE.start(),
            last: *DEFAULT_RANGE.end(),
            per_session: DEFAULT_PER_SESSION,
        }
    }
}

#[derive(Default)]
struct State {
    by_session: BTreeMap<String, Vec<u16>>,
    holder: BTreeMap<u16, String>,
    cursor: Option<u16>,
    claims: BTreeMap<u16, Claim>,
    next_claim: u64,
}

/// The lease table. Thread-safe; share it by reference or inside an `Arc`.
pub struct PortLeases {
    config: LeaseConfig,
    state: Arc<Mutex<State>>,
}

/// A port reserved before spawning. Dropping it releases a failed start; recording a process keeps it
/// reserved until an identity check proves that process exited. A tool timeout does neither.
#[must_use = "keep the reservation until the start fails or record its process identity"]
pub struct PortReservation {
    state: Arc<Mutex<State>>,
    port: u16,
    id: u64,
    recorded: bool,
    fallback: Option<ProcessRef>,
}

impl PortReservation {
    /// The port to pass as `PORT`.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Associates a successful start with its OS identity before acknowledging it.
    pub fn record(mut self, process: ProcessRef) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(claim) = state.claims.get_mut(&self.port).filter(|c| c.id == self.id) {
            claim.process = Some(process);
        }
        self.recorded = true;
    }
}

impl Drop for PortReservation {
    fn drop(&mut self) {
        if !self.recorded {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state
                .claims
                .get(&self.port)
                .is_some_and(|c| c.id == self.id)
            {
                if let Some(process) = self.fallback.filter(|p| crate::os::is_alive(*p)) {
                    if let Some(claim) = state.claims.get_mut(&self.port) {
                        claim.process = Some(process);
                    }
                    return;
                }
                state.claims.remove(&self.port);
                remove_port(&mut state, self.port);
            }
        }
    }
}

fn remove_port(state: &mut State, port: u16) {
    if let Some(session) = state.holder.remove(&port)
        && let Some(ports) = state.by_session.get_mut(&session)
    {
        ports.retain(|p| *p != port);
        if ports.is_empty() {
            state.by_session.remove(&session);
        }
    }
}

/// True when an actual bind of `127.0.0.1:<port>` is refused right now. `TIME_WAIT` alone does
/// not decide availability: socket reuse and Windows bind rules can leave that port usable.
#[must_use]
pub fn bind_refused(port: u16) -> bool {
    TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).is_err()
}

impl PortLeases {
    /// An empty lease table over `config`'s range.
    #[must_use]
    pub fn new(config: LeaseConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &LeaseConfig {
        &self.config
    }

    /// The session's lease, topped up to `per_session` ports. Ports already leased to it are kept as they
    /// are while usable or reserved for a start/live process. An unclaimed port with no listener but a
    /// refused bind is returned and replaced (`TIME_WAIT`, excluded ranges). `busy` is the socket-table check.
    ///
    /// # Errors
    /// [`LeaseError::RangeExhausted`] when the range has too few free ports; the ports found are kept for the
    /// session (a partial lease beats none) and the error says how many.
    pub fn lease(
        &self,
        session_id: &str,
        busy: &dyn Fn(u16) -> bool,
    ) -> Result<Vec<u16>, LeaseError> {
        let mut state = self.lock();
        let unavailable: Vec<u16> = state
            .by_session
            .get(session_id)
            .into_iter()
            .flatten()
            .copied()
            .filter(|p| !state.claims.contains_key(p) && !busy(*p) && bind_refused(*p))
            .collect();
        for port in unavailable {
            remove_port(&mut state, port);
        }
        let have = state.by_session.get(session_id).map_or(0, Vec::len);
        let wanted = self.config.per_session.saturating_sub(have);
        let span = usize::from(self.config.last.saturating_sub(self.config.first)) + 1;
        let mut port = state
            .cursor
            .map_or(self.config.first, |c| self.next_after(c));
        let mut found = Vec::new();
        for _ in 0..span {
            if found.len() == wanted {
                break;
            }
            if !state.holder.contains_key(&port) && !busy(port) && !bind_refused(port) {
                found.push(port);
            }
            port = self.next_after(port);
        }
        if let Some(last) = found.last() {
            state.cursor = Some(*last);
        }
        for port in &found {
            state.holder.insert(*port, session_id.to_owned());
        }
        let lease = state.by_session.entry(session_id.to_owned()).or_default();
        lease.extend(found.iter().copied());
        let total = lease.clone();
        if found.len() < wanted {
            return Err(LeaseError::RangeExhausted {
                start: self.config.first,
                end: self.config.last,
                wanted,
                found: found.len(),
            });
        }
        Ok(total)
    }

    /// Atomically reserves an available port already leased to this session, before starting a process.
    ///
    /// # Errors
    /// A port outside this session's lease or no available port. OS bind refusals and process reservations
    /// count as unavailable even when no listener appears in the socket table.
    #[must_use = "keep the reservation until the start fails or record its process identity"]
    pub fn reserve(
        &self,
        session_id: &str,
        asked: Option<u16>,
        busy: &dyn Fn(u16) -> bool,
    ) -> Result<PortReservation, LeaseError> {
        let mut state = self.lock();
        let leased = state
            .by_session
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        let available = |p: u16| !state.claims.contains_key(&p) && !busy(p) && !bind_refused(p);
        let port = if let Some(port) = asked {
            if !leased.contains(&port) {
                return Err(LeaseError::NotLeased { port, leased });
            }
            if !available(port) {
                return Err(LeaseError::Unavailable { port, leased });
            }
            port
        } else {
            leased
                .iter()
                .copied()
                .find(|p| available(*p))
                .ok_or_else(|| LeaseError::AllUnavailable {
                    leased: leased.clone(),
                })?
        };
        state.next_claim = state.next_claim.wrapping_add(1);
        let id = state.next_claim;
        state.claims.insert(port, Claim { id, process: None });
        Ok(PortReservation {
            state: self.state.clone(),
            port,
            id,
            recorded: false,
            fallback: None,
        })
    }

    /// Keeps a process's original port atomically reserved while stopping and restarting it. On a
    /// failed stop, dropping the guard restores the original live claim instead of losing its port.
    ///
    /// # Errors
    /// The port is no longer this session's, or another pending/live start has claimed it.
    #[must_use = "keep the original port reserved until restart finishes"]
    pub fn reserve_restart(
        &self,
        session_id: &str,
        port: u16,
        process: ProcessRef,
    ) -> Result<PortReservation, LeaseError> {
        let mut state = self.lock();
        let leased = state
            .by_session
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        if state.holder.get(&port).is_none_or(|s| s != session_id) {
            return Err(LeaseError::NotLeased { port, leased });
        }
        if state
            .claims
            .get(&port)
            .is_some_and(|c| c.process != Some(process))
        {
            return Err(LeaseError::Unavailable { port, leased });
        }
        state.next_claim = state.next_claim.wrapping_add(1);
        let id = state.next_claim;
        state.claims.insert(port, Claim { id, process: None });
        Ok(PortReservation {
            state: self.state.clone(),
            port,
            id,
            recorded: false,
            fallback: Some(process),
        })
    }

    /// Assigned ports of this exact live process, including before it listens.
    #[must_use]
    pub fn ports_of(&self, process: ProcessRef) -> Vec<u16> {
        self.lock()
            .claims
            .iter()
            .filter_map(|(port, claim)| (claim.process == Some(process)).then_some(*port))
            .collect()
    }

    /// Whether a start or live process has claimed this port, including before it listens.
    #[must_use]
    pub fn reserved(&self, port: u16) -> bool {
        self.lock().claims.contains_key(&port)
    }

    /// Releases claims whose recorded process exited, using a fresh identity check. Pending starts stay
    /// reserved; a scan taken before a spawn must never decide that the newly recorded process is gone.
    pub fn reconcile_processes(&self, alive: impl Fn(ProcessRef) -> bool) {
        let mut state = self.lock();
        let gone: Vec<u16> = state
            .claims
            .iter()
            .filter_map(|(port, claim)| claim.process.filter(|p| !alive(*p)).map(|_| *port))
            .collect();
        for port in gone {
            state.claims.remove(&port);
            remove_port(&mut state, port);
        }
    }

    /// Restores a live process's port from its launch record, also before it begins listening.
    #[must_use]
    pub fn adopt_process(&self, session_id: &str, port: u16, process: ProcessRef) -> bool {
        if !self.adopt(session_id, port) {
            return false;
        }
        let mut state = self.lock();
        if state.holder.get(&port).is_none_or(|s| s != session_id) {
            return false;
        }
        state.claims.entry(port).or_insert(Claim {
            id: 0,
            process: Some(process),
        });
        true
    }

    fn next_after(&self, port: u16) -> u16 {
        if port >= self.config.last || port < self.config.first {
            self.config.first
        } else {
            port + 1
        }
    }

    /// Gives `port` to `session_id` without checks (a re-attached process of that session holds it). Ignored
    /// when the port is outside the range or another session holds it. Returns whether it was adopted.
    #[must_use]
    pub fn adopt(&self, session_id: &str, port: u16) -> bool {
        if !(self.config.first..=self.config.last).contains(&port) {
            return false;
        }
        let mut state = self.lock();
        match state.holder.get(&port) {
            Some(holder) if holder != session_id => false,
            Some(_) => true,
            None => {
                state.holder.insert(port, session_id.to_owned());
                state
                    .by_session
                    .entry(session_id.to_owned())
                    .or_default()
                    .push(port);
                true
            }
        }
    }

    /// The session's current lease (empty when none).
    #[must_use]
    pub fn of(&self, session_id: &str) -> Vec<u16> {
        self.lock()
            .by_session
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Which session holds `port`, if any.
    #[must_use]
    pub fn holder(&self, port: u16) -> Option<String> {
        self.lock().holder.get(&port).cloned()
    }

    /// Returns unused session ports to the pool. Pending starts and live processes keep their claims
    /// and remain reported, including after session end; exit reconciliation releases them later.
    #[must_use]
    pub fn release(&self, session_id: &str) -> Vec<u16> {
        let mut state = self.lock();
        let ports: Vec<u16> = state
            .by_session
            .get(session_id)
            .into_iter()
            .flatten()
            .copied()
            .filter(|p| !state.claims.contains_key(p))
            .collect();
        for port in &ports {
            remove_port(&mut state, *port);
        }
        ports
    }

    /// Gives session `old`'s ports to `new` (Vibe handed the conversation to a new session id); `new`
    /// keeps any ports it already had. Returns the ports that moved.
    #[must_use = "the moved items tell what followed the hand-off"]
    pub fn hand_off(&self, old: &str, new: &str) -> Vec<u16> {
        let mut state = self.lock();
        let ports = state.by_session.remove(old).unwrap_or_default();
        for port in &ports {
            state.holder.insert(*port, new.to_owned());
        }
        let held = state.by_session.entry(new.to_owned()).or_default();
        held.extend(ports.iter().copied());
        if held.is_empty() {
            state.by_session.remove(new);
        }
        ports
    }

    /// Returns unused slots of sessions not in `live` (after a restart or a missed end); returns the
    /// sessions reconciled. Pending/live process claims remain until failure or exit.
    #[must_use]
    pub fn reconcile(&self, live: &[String]) -> Vec<String> {
        let gone: Vec<String> = self
            .lock()
            .by_session
            .keys()
            .filter(|s| !live.contains(s))
            .cloned()
            .collect();
        for session in &gone {
            let _ = self.release(session);
        }
        gone
    }

    /// Every session's lease.
    #[must_use]
    pub fn all(&self) -> BTreeMap<String, Vec<u16>> {
        self.lock().by_session.clone()
    }
}
