//! Every listening port of every [`crate::NetGuard`] in this process: the shared listener and each store listener.
//! A guard enters its ports when it binds and when its listeners change, and leaves when it shuts down or is dropped,
//! so no guard needs to opt in. Each guard refuses these ports like its own (Vibe Work's own ports), and the browser
//! content rules hold them for every page ([`crate::rules::current_inputs`]); a change moves [`crate::rules::revision`].
//!
//! Every other loopback listener of Vibe Work (Debug API, key proxy, agent tools, page servers, ...) binds through
//! [`bind_own`] or [`bind_own_tokio`]: the port enters the same set before the listener is handed out, and leaves
//! when the returned [`OwnPort`] is dropped. So a new listener is refused from the first moment without a separate
//! registration step (`tests/own_bind_sites.rs` fails for a product bind that does not go through here).
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

/// Ports per guard (by [`next_id`]).
static LIVE: Mutex<BTreeMap<u64, Vec<u16>>> = Mutex::new(BTreeMap::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

/// A new guard's id in the set.
pub(crate) fn next_id() -> u64 {
    NEXT.fetch_add(1, Ordering::SeqCst)
}

/// Sets the guard `id`'s listening ports (empty = it has none now).
pub(crate) fn set(id: u64, mut ports: Vec<u16>) {
    ports.sort_unstable();
    ports.dedup();
    let mut live = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
    let changed = if ports.is_empty() {
        live.remove(&id).is_some()
    } else {
        live.insert(id, ports.clone()).as_ref() != Some(&ports)
    };
    drop(live);
    if changed {
        crate::rules::inputs_changed();
    }
}

/// True when `port` is a listener of any guard in this process.
pub(crate) fn contains(port: u16) -> bool {
    LIVE.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .any(|ports| ports.contains(&port))
}

/// Every guard's listening ports now, sorted.
#[must_use]
pub fn ports() -> Vec<u16> {
    let mut all: Vec<u16> = LIVE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .flatten()
        .copied()
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}

/// A loopback port of Vibe Work itself, held in the process-wide set while this value lives: every guard refuses it
/// and the content rules hold it for every page. Keep it as long as the listener (drop it with the listener).
#[derive(Debug)]
pub struct OwnPort {
    id: u64,
    port: u16,
}

impl OwnPort {
    /// The held port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for OwnPort {
    fn drop(&mut self) {
        set(self.id, Vec::new());
    }
}

fn hold(port: u16) -> OwnPort {
    let id = next_id();
    set(id, vec![port]);
    OwnPort { id, port }
}

/// Binds a listener of Vibe Work itself on `127.0.0.1` with an OS-chosen port; the port is in the own-listener set
/// before this returns (blocking std listener).
///
/// # Errors
/// The bind error.
pub fn bind_own() -> std::io::Result<(std::net::TcpListener, OwnPort)> {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let own = hold(listener.local_addr()?.port());
    Ok((listener, own))
}

/// [`bind_own`] for a tokio listener.
///
/// # Errors
/// The bind error.
pub async fn bind_own_tokio() -> std::io::Result<(tokio::net::TcpListener, OwnPort)> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let own = hold(listener.local_addr()?.port());
    Ok((listener, own))
}
