//! Main-document address evidence from successful pinned proxy connections, frozen at native commit.
//! Depends on netguard's IP classes and the mount registry; called by the guard context, page hooks and capture.
//! Frames can add connection evidence but never change a committed document's class. Ambiguity fails closed.

use std::collections::HashMap;
use std::net::SocketAddr;

use vw_netguard::ip::{IpClass, classify};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AddressClass {
    Public,
    Private,
    Mixed,
}

fn class(addresses: &[SocketAddr]) -> AddressClass {
    if !addresses.is_empty()
        && addresses
            .iter()
            .all(|a| matches!(classify(a.ip()), IpClass::Private | IpClass::SharedCgnat))
    {
        AddressClass::Private
    } else if !addresses.is_empty()
        && addresses
            .iter()
            .all(|a| classify(a.ip()) == IpClass::Public)
    {
        AddressClass::Public
    } else {
        AddressClass::Mixed
    }
}

fn host_key(host: &str) -> String {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

/// Retain evidence for pooled connections, never replacing public evidence with a later private answer.
#[derive(Clone, Debug, Default)]
pub(crate) struct LoadAddresses {
    connections: HashMap<(String, u16), AddressClass>,
    exhausted: bool,
    committed: Option<AddressClass>,
    generation: u64,
}

impl LoadAddresses {
    fn connected(&mut self, host: &str, port: u16, addresses: &[SocketAddr]) {
        if self.exhausted {
            return;
        }
        let key = (host_key(host), port);
        if self.connections.len() >= 128 && !self.connections.contains_key(&key) {
            self.connections.clear();
            self.exhausted = true;
            return;
        }
        let next = class(addresses);
        self.connections
            .entry(key)
            .and_modify(|old| {
                if *old != next {
                    *old = AddressClass::Mixed;
                }
            })
            .or_insert(next);
    }

    pub(crate) fn commit(&mut self, url: &url::Url) {
        self.generation = self.generation.wrapping_add(1);
        // A new main document always replaces the previous grant, including unproven/cached loads.
        self.committed = url
            .host_str()
            .zip(url.port_or_known_default())
            .and_then(|(host, port)| self.connections.get(&(host_key(host), port)).copied());
    }

    pub(crate) fn private_generation(&self) -> Option<u64> {
        (self.committed == Some(AddressClass::Private)).then_some(self.generation)
    }
}

impl crate::PreviewService {
    /// Host-only receipt of addresses the egress proxy successfully connected with, never page-reported DNS.
    /// Stale mounts are ignored; only a later main-document commit can use this evidence.
    pub fn record_connected_addresses(
        &self,
        label: &str,
        host: &str,
        port: u16,
        addresses: &[SocketAddr],
    ) {
        let mut state = self.inner.lock();
        let Some(session) = state.session_of(label).map(str::to_owned) else {
            return;
        };
        if let Some(mount) = state
            .entry(&session)
            .mount
            .as_mut()
            .filter(|m| m.label == label)
        {
            mount
                .navigation
                .load_addresses
                .connected(host, port, addresses);
        }
    }
}
