//! A refused loopback endpoint for offline tests. Called by the installer and proxy test rigs.
//! Depends on the runtime's socket API; owns a bound socket until the caller finishes using its address.

use std::net::{Ipv4Addr, SocketAddr};

/// Holds a port without listening, so an unrelated server cannot take the offline endpoint.
pub struct ClosedPort {
    socket: tokio::net::TcpSocket,
}

impl ClosedPort {
    /// Reserves an OS-assigned loopback port until this guard is dropped.
    pub fn bind() -> Self {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
        // Linux refuses connect() to a bound, non-listening socket. A listening socket would instead
        // complete the handshake and hang the HTTP request. Windows/macOS behavior is not verified here.
        Self { socket }
    }

    /// The held address; borrowing the guard does not release its reservation.
    pub fn addr(&self) -> SocketAddr {
        self.socket.local_addr().unwrap()
    }

    /// An HTTP URL at the held address with the requested absolute path.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr())
    }
}

/// Models a foreign server taking the historical picker's released port for negative regression tests.
/// Retries if a real concurrent process wins the intentional gap; the returned listener stays owned.
pub async fn occupy_released_port() -> tokio::net::TcpListener {
    for _ in 0..100 {
        let probe = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => return listener,
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(error) => panic!("foreign fixture cannot bind {addr}: {error}"),
        }
    }
    panic!("foreign fixture lost every intentional takeover attempt");
}
