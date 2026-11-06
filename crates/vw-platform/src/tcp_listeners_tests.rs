//! Synthetic Windows owner-PID table regressions. Calls the native decoder on Linux too;
//! checks both address families, anomalous state values, multiple rows and truncated buffers.

#![allow(clippy::unwrap_used)]
use super::*;

fn table(family: Family, state: u32) -> Vec<u32> {
    let port = u32::from(28100_u16.to_be());
    match family {
        Family::V4 => vec![
            1,
            state,
            u32::from_ne_bytes([127, 0, 0, 1]),
            port,
            0,
            0,
            4242,
        ],
        Family::V6 => vec![
            1,
            0,
            0,
            0,
            u32::from_ne_bytes([0, 0, 0, 1]),
            0,
            port,
            0,
            0,
            0,
            0,
            0,
            0,
            state,
            4242,
        ],
    }
}

#[cfg(windows)]
#[test]
fn native_tables_include_listeners_but_never_connected_client_ports() {
    for address in ["127.0.0.1:0", "[::1]:0"] {
        let server = std::net::TcpListener::bind(address).unwrap();
        let bound = server.local_addr().unwrap();
        let client = std::net::TcpStream::connect(bound).unwrap();
        let (_accepted, _) = server.accept().unwrap();
        let client_port = client.local_addr().unwrap().port();
        let rows = windows_tcp_listeners().unwrap();
        assert!(
            rows.iter()
                .any(|row| row.port == bound.port() && row.pid == std::process::id())
        );
        assert!(!rows.iter().any(|row| row.port == client_port));
        drop(server);
        let rows = windows_tcp_listeners().unwrap();
        assert!(
            !rows
                .iter()
                .any(|row| row.port == bound.port() && row.pid == std::process::id())
        );
    }
}

#[test]
fn listener_table_membership_survives_inconsistent_tcp_state() {
    for family in [Family::V4, Family::V6] {
        for state in [6, 0, 1, 2, 5, 11] {
            let found = decode(&table(family, state), family).unwrap();
            assert_eq!(found.len(), 1, "listener-only row lost at state {state}");
            assert_eq!(found[0].pid, 4242);
            assert_eq!(found[0].port, 28100);
            assert!(found[0].address.is_loopback());
        }
    }
}

#[test]
fn listener_table_preserves_competing_owners_and_rejects_truncation() {
    for family in [Family::V4, Family::V6] {
        let mut first = table(family, 2);
        let mut second = table(family, 2);
        *second.last_mut().unwrap() = 4343;
        first[0] = 2;
        first.extend_from_slice(&second[1..]);
        let found = decode(&first, family).unwrap();
        assert_eq!(
            found.iter().map(|row| row.pid).collect::<Vec<_>>(),
            [4242, 4343]
        );
        first.pop();
        assert!(decode(&first, family).is_err());
        assert!(decode(&[], family).is_err());
        assert!(decode(&[u32::MAX], family).is_err());
        assert_eq!(decode(&[0], family).unwrap(), []);
    }
}
