//! Windows owner-PID listener tables for procmon. Uses IP Helper's listener-only class,
//! with a bounds-checked decoder shared by native scans and synthetic tests on every OS.
//! Table membership supplies readiness; process identity and session attribution remain in procmon.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// One TCP endpoint returned by Windows' listener-only owner-PID table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TcpListener {
    /// Local bind address (possibly a wildcard).
    pub address: IpAddr,
    /// Local port in host order.
    pub port: u16,
    /// OS-reported owning PID, including processes we cannot inspect.
    pub pid: u32,
}

#[derive(Clone, Copy)]
enum Family {
    V4,
    V6,
}

// MIB_TCPTABLE_OWNER_PID / MIB_TCP6TABLE_OWNER_PID: DWORD count followed by rows.
// Native compilation checks these offsets against the SDK bindings below.
fn decode(table: &[u32], family: Family) -> io::Result<Vec<TcpListener>> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "truncated TCP listener table");
    let (&count, rows) = table.split_first().ok_or_else(invalid)?;
    let stride = match family {
        Family::V4 => 6,
        Family::V6 => 14,
    };
    let count = usize::try_from(count).map_err(|_| invalid())?;
    if count > rows.len() / stride {
        return Err(invalid());
    }
    let mut listeners = Vec::with_capacity(count);
    for row in rows.chunks_exact(stride).take(count) {
        let (address, port, pid) = match family {
            Family::V4 => (
                IpAddr::V4(Ipv4Addr::from(row[1].to_ne_bytes())),
                row[2],
                row[5],
            ),
            Family::V6 => {
                let mut address = [0; 16];
                for (bytes, word) in address.as_chunks_mut::<4>().0.iter_mut().zip(&row[..4]) {
                    bytes.copy_from_slice(&word.to_ne_bytes());
                }
                (IpAddr::V6(Ipv6Addr::from(address)), row[5], row[13])
            }
        };
        // Only TCP_TABLE_OWNER_PID_LISTENER may feed this decoder. Windows can return
        // inconsistent dwState values; membership in this OS-filtered table selects listeners.
        // Never apply this rule to TCP_TABLE_OWNER_PID_ALL or a connection table.
        listeners.push(TcpListener {
            address,
            port: u16::from_be(u16::try_from(port & 0xffff).map_err(|_| invalid())?),
            pid,
        });
    }
    Ok(listeners)
}

/// IPv4 and IPv6 TCP listeners, with the OS owning PID, without connecting to any endpoint.
///
/// # Errors
/// The native error (including access denied), a malformed table, or persistent table growth.
#[cfg(windows)]
pub fn windows_tcp_listeners() -> io::Result<Vec<TcpListener>> {
    let mut listeners = decode(&native::table(Family::V4)?, Family::V4)?;
    listeners.extend(decode(&native::table(Family::V6)?, Family::V6)?);
    Ok(listeners)
}

#[cfg(windows)]
mod native {
    #![allow(unsafe_code)]

    use super::{Family, io};
    use std::mem::{align_of, offset_of, size_of};
    use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
        MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    // Reject an SDK layout change at compile time. DWORD storage provides the required alignment;
    // no unchecked row pointers or assumptions about Vec<u8> alignment enter the decoder.
    const _: () = {
        assert!(align_of::<MIB_TCPTABLE_OWNER_PID>() == align_of::<u32>());
        assert!(align_of::<MIB_TCP6TABLE_OWNER_PID>() == align_of::<u32>());
        assert!(offset_of!(MIB_TCPTABLE_OWNER_PID, table) == 4);
        assert!(offset_of!(MIB_TCP6TABLE_OWNER_PID, table) == 4);
        assert!(size_of::<MIB_TCPROW_OWNER_PID>() == 24);
        assert!(offset_of!(MIB_TCPROW_OWNER_PID, dwLocalAddr) == 4);
        assert!(offset_of!(MIB_TCPROW_OWNER_PID, dwLocalPort) == 8);
        assert!(offset_of!(MIB_TCPROW_OWNER_PID, dwOwningPid) == 20);
        assert!(size_of::<MIB_TCP6ROW_OWNER_PID>() == 56);
        assert!(offset_of!(MIB_TCP6ROW_OWNER_PID, ucLocalAddr) == 0);
        assert!(offset_of!(MIB_TCP6ROW_OWNER_PID, dwLocalPort) == 20);
        assert!(offset_of!(MIB_TCP6ROW_OWNER_PID, dwOwningPid) == 52);
    };

    pub(super) fn table(family: Family) -> io::Result<Vec<u32>> {
        let af = match family {
            Family::V4 => AF_INET,
            Family::V6 => AF_INET6,
        };
        let mut buffer: Vec<u32> = Vec::new();
        for _ in 0..8 {
            let mut bytes = u32::try_from(buffer.len() * 4).map_err(io::Error::other)?;
            // SAFETY: the optional buffer is initialized, DWORD-aligned and writable for `bytes`;
            // `bytes` is a writable DWORD. The family/class select the SDK-checked layouts above.
            let status = unsafe {
                GetExtendedTcpTable(
                    if buffer.is_empty() {
                        None
                    } else {
                        Some(buffer.as_mut_ptr().cast())
                    },
                    &raw mut bytes,
                    false,
                    u32::from(af.0),
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            if status == 0 {
                let words = usize::try_from(bytes)
                    .map_err(io::Error::other)?
                    .div_ceil(4);
                if words > buffer.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "TCP table exceeds buffer",
                    ));
                }
                buffer.truncate(words);
                return Ok(buffer);
            }
            if status != ERROR_INSUFFICIENT_BUFFER.0 {
                return Err(io::Error::from_raw_os_error(
                    i32::try_from(status).map_err(io::Error::other)?,
                ));
            }
            if bytes > 16 * 1024 * 1024 {
                return Err(io::Error::other("TCP listener table exceeds 16 MiB"));
            }
            buffer.resize(
                usize::try_from(bytes)
                    .map_err(io::Error::other)?
                    .div_ceil(4),
                0,
            );
        }
        Err(io::Error::other(
            "TCP listener table kept growing; retry the scan",
        ))
    }
}

#[cfg(test)]
#[path = "tcp_listeners_tests.rs"]
mod tests;
