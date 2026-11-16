//! This computer's own interface addresses: every IPv4 and IPv6 unicast address of every interface (up or not), as
//! the OS lists them now. The browser's network guard counts each as this computer (vw-netguard `own`).
//!
//! Depends on: libc `getifaddrs` (Linux, macOS), IP Helper `GetAdaptersAddresses` (Windows). Called by: vw-core-api
//! (reads the list again every few seconds and hands a changed list to the guard).
//! Invariants:
//! - Read-only and bounded: one OS call (Windows: at most a few, when the buffer grows), no network traffic.
//! - Every address the OS reports is returned, loopback included; the caller decides what to keep.

use std::io;
use std::net::IpAddr;

/// This computer's interface addresses now (see the module docs).
///
/// # Errors
/// The OS call failed.
pub fn interface_addresses() -> io::Result<Vec<IpAddr>> {
    imp::read()
}

/// The address in a raw socket address (`bytes` = the OS's `sockaddr` of `family`), when it is IPv4 or IPv6.
/// Windows layout (no length byte): family (2 bytes), port (2), then the IPv4 address; for IPv6 the flow info (4),
/// then the 16-byte address.
#[cfg(any(windows, test))]
fn from_windows_sockaddr(bytes: &[u8]) -> Option<IpAddr> {
    let family = u16::from_ne_bytes([*bytes.first()?, *bytes.get(1)?]);
    match family {
        2 => {
            let octets: [u8; 4] = bytes.get(4..8)?.try_into().ok()?;
            Some(IpAddr::from(octets))
        }
        23 => {
            let octets: [u8; 16] = bytes.get(8..24)?.try_into().ok()?;
            Some(IpAddr::from(octets))
        }
        _ => None,
    }
}

#[cfg(unix)]
mod imp {
    use std::io;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    pub(super) fn read() -> io::Result<Vec<IpAddr>> {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        // SAFETY: `getifaddrs` fills `head` with a list that stays valid until `freeifaddrs`, called below.
        if unsafe { libc::getifaddrs(&raw mut head) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut addresses = Vec::new();
        let mut node = head;
        while !node.is_null() {
            // SAFETY: `node` is a node of the list `getifaddrs` returned; it is not freed before the loop ends.
            let entry = unsafe { &*node };
            let address = entry.ifa_addr;
            if !address.is_null() {
                // SAFETY: a non-null `ifa_addr` points to a socket address whose family field is valid.
                let family = i32::from(unsafe { (*address).sa_family });
                if family == libc::AF_INET {
                    // SAFETY: the family says this is a `sockaddr_in`; read unaligned, copied out.
                    let sin = unsafe { address.cast::<libc::sockaddr_in>().read_unaligned() };
                    addresses.push(IpAddr::V4(Ipv4Addr::from(u32::from_be(
                        sin.sin_addr.s_addr,
                    ))));
                } else if family == libc::AF_INET6 {
                    // SAFETY: the family says this is a `sockaddr_in6`; read unaligned, copied out.
                    let sin6 = unsafe { address.cast::<libc::sockaddr_in6>().read_unaligned() };
                    addresses.push(IpAddr::V6(Ipv6Addr::from(sin6.sin6_addr.s6_addr)));
                }
            }
            node = entry.ifa_next;
        }
        // SAFETY: `head` came from `getifaddrs` and is freed once; no reference into the list outlives this call.
        unsafe { libc::freeifaddrs(head) };
        Ok(addresses)
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::net::IpAddr;
    use windows::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
        GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
    };

    /// `ERROR_BUFFER_OVERFLOW`: the list grew between the size query and the read.
    const BUFFER_OVERFLOW: u32 = 111;
    /// `ERROR_NO_DATA`: no adapter has an address.
    const NO_DATA: u32 = 232;

    pub(super) fn read() -> io::Result<Vec<IpAddr>> {
        // u64 cells: the adapter records need 8-byte alignment.
        let mut size: u32 = 16 * 1024;
        for _ in 0..4 {
            let mut buffer = vec![0_u64; (size as usize).div_ceil(8)];
            let first = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
            // SAFETY: `first` points to `size` writable, 8-byte aligned bytes; the OS writes at most that many and
            // updates `size` when it needs more.
            let status = unsafe {
                GetAdaptersAddresses(
                    0,
                    GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER,
                    None,
                    Some(first),
                    &raw mut size,
                )
            };
            match status {
                0 => return Ok(collect(first)),
                NO_DATA => return Ok(Vec::new()),
                BUFFER_OVERFLOW => {}
                code => {
                    return Err(io::Error::from_raw_os_error(
                        i32::try_from(code).unwrap_or(i32::MAX),
                    ));
                }
            }
        }
        Err(io::Error::other("the adapter list kept growing"))
    }

    fn collect(first: *const IP_ADAPTER_ADDRESSES_LH) -> Vec<IpAddr> {
        let mut addresses = Vec::new();
        let mut adapter = first;
        while !adapter.is_null() {
            // SAFETY: `adapter` is a record of the list the OS just wrote into the caller's live buffer.
            let record = unsafe { &*adapter };
            let mut unicast = record.FirstUnicastAddress;
            while !unicast.is_null() {
                // SAFETY: a unicast record of the same list.
                let entry = unsafe { &*unicast };
                let raw = entry.Address.lpSockaddr;
                let len = usize::try_from(entry.Address.iSockaddrLength).unwrap_or(0);
                if !raw.is_null() && len > 0 {
                    // SAFETY: the OS reports `len` valid bytes of socket address at `raw`.
                    let bytes = unsafe { std::slice::from_raw_parts(raw.cast::<u8>(), len) };
                    if let Some(ip) = super::from_windows_sockaddr(bytes) {
                        addresses.push(ip);
                    }
                }
                unicast = entry.Next;
            }
            adapter = record.Next;
        }
        addresses
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn windows_socket_addresses_are_decoded() {
        let v4 = [2, 0, 0x1f, 0x90, 198, 51, 100, 38, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            from_windows_sockaddr(&v4),
            Some("198.51.100.38".parse().unwrap())
        );
        let mut v6 = vec![23, 0, 0, 80, 0, 0, 0, 0];
        v6.extend_from_slice(&"fd00::5".parse::<std::net::Ipv6Addr>().unwrap().octets());
        v6.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(from_windows_sockaddr(&v6), Some("fd00::5".parse().unwrap()));
        assert_eq!(from_windows_sockaddr(&v6[..20]), None, "truncated");
        assert_eq!(from_windows_sockaddr(&[17, 0, 1, 2]), None, "other family");
    }

    #[cfg(unix)]
    #[test]
    fn this_computer_lists_its_loopback_address() {
        let addresses = interface_addresses().unwrap();
        assert!(addresses.iter().any(IpAddr::is_loopback), "{addresses:?}");
        // The source address of the default route (no packet is sent) is one of them.
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        if socket.connect("192.0.2.1:9").is_ok() {
            let route = socket.local_addr().unwrap().ip();
            assert!(addresses.contains(&route), "{route} not in {addresses:?}");
        }
    }
}
