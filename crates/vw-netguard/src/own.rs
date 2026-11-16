//! This computer's own interface addresses (every IPv4 and IPv6 address of every interface, as the app last read
//! them): a request to one of them reaches this computer, whatever class the address has, so the guard treats it
//! like loopback (Vibe Work's own ports are refused on it, and a page from the web is asked about "this computer").
//!
//! Depends on: std, `rules` (a change moves the content rules' revision). Called by: the app (gives the list read from
//! the OS, again whenever it changes), `decide` (the class of a target), `rules` (the macOS content rules: `WebKit`
//! sends this computer's own address around the guard's proxy).
//! Invariants:
//! - Pure state: the list is what the app last gave ([`set_own_addresses`]); nothing here reads the OS.
//! - Loopback and unspecified addresses are not listed (they are loopback by class already).

use std::net::IpAddr;
use std::sync::{PoisonError, RwLock};

/// The addresses ([`set_own_addresses`]), sorted, no duplicates.
static OWN: RwLock<Vec<IpAddr>> = RwLock::new(Vec::new());

/// Sets this computer's own interface addresses (the full list read from the OS now). Returns true when the list
/// changed (the content rules' revision then moved, so mounted pages get their lists again).
pub fn set_own_addresses(addresses: Vec<IpAddr>) -> bool {
    let mut addresses: Vec<IpAddr> = addresses
        .into_iter()
        .map(|ip| ip.to_canonical())
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .collect();
    addresses.sort_unstable();
    addresses.dedup();
    let mut held = OWN.write().unwrap_or_else(PoisonError::into_inner);
    if *held == addresses {
        return false;
    }
    *held = addresses;
    drop(held);
    crate::rules::inputs_changed();
    true
}

/// This computer's own interface addresses as last given.
#[must_use]
pub fn own_addresses() -> Vec<IpAddr> {
    OWN.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// True when `ip` (IPv4-mapped IPv6 counts as its IPv4 address) is one of this computer's own interface addresses.
#[must_use]
pub fn is_own(ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    OWN.read()
        .unwrap_or_else(PoisonError::into_inner)
        .binary_search(&ip)
        .is_ok()
}
