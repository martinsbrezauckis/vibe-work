//! Pure parser for Linux `/proc/net/tcp` and `/proc/net/tcp6`: the listening sockets of this network
//! namespace with their owner uid and inode (the inode is what `/proc/<pid>/fd/*` links point at).
//!
//! Depends on: std only. Called by: `os::linux` (with the real files), tests (with synthetic lines).
//! Format (kernel `net/ipv4/tcp_ipv4.c` `get_tcp4_sock`): `sl local_address rem_address st tx:rx tr:when
//! retrnsmt uid timeout inode ...`; addresses are hex in host byte order per 32-bit word, ports hex; state
//! `0A` is LISTEN. A malformed line is skipped, never an error.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// One LISTEN row of `/proc/net/tcp{,6}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcNetListener {
    /// Local address (IPv4-mapped IPv6 addresses are returned as IPv4).
    pub address: IpAddr,
    /// Local port.
    pub port: u16,
    /// Owner uid of the socket.
    pub uid: u32,
    /// Socket inode (`socket:[<inode>]` in `/proc/<pid>/fd`).
    pub inode: u64,
}

/// TCP state `LISTEN` in the kernel's table.
const LISTEN: &str = "0A";

/// Parses the full text of `/proc/net/tcp` or `/proc/net/tcp6` and returns its LISTEN rows. The header line
/// and malformed lines are skipped.
#[must_use]
pub fn parse_proc_net_tcp(text: &str) -> Vec<ProcNetListener> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<ProcNetListener> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    // sl, local, remote, st, tx:rx, tr:when, retrnsmt, uid, timeout, inode
    if fields.len() < 10 || fields.get(3).copied() != Some(LISTEN) {
        return None;
    }
    let (address, port) = parse_endpoint(fields.get(1)?)?;
    let uid = fields.get(7)?.parse().ok()?;
    let inode = fields.get(9)?.parse().ok()?;
    Some(ProcNetListener {
        address,
        port,
        uid,
        inode,
    })
}

/// `0100007F:1F90` -> `127.0.0.1:8080`; 32 hex digits for IPv6.
fn parse_endpoint(text: &str) -> Option<(IpAddr, u16)> {
    let (address, port) = text.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let address = match address.len() {
        8 => IpAddr::V4(word_to_v4(u32::from_str_radix(address, 16).ok()?)),
        32 => {
            let mut bytes = [0u8; 16];
            for (i, chunk) in bytes.chunks_mut(4).enumerate() {
                let word = u32::from_str_radix(address.get(i * 8..i * 8 + 8)?, 16).ok()?;
                // Each 32-bit word is printed in host order (little-endian on every platform we ship).
                chunk.copy_from_slice(&word.to_le_bytes());
            }
            let v6 = Ipv6Addr::from(bytes);
            v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4)
        }
        _ => return None,
    };
    Some((address, port))
}

fn word_to_v4(word: u32) -> Ipv4Addr {
    Ipv4Addr::from(word.to_le_bytes())
}
