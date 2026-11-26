//! Checked service streams use the same manual and PAC routes as the HTTP relay.
#![cfg(feature = "proxy-transports")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use vw_netguard::upstream::{Error, ProxyResolver};
use vw_netguard::{NetGuard, Resolve, ResolveFuture, ServiceStreamError, Upstream};
#[path = "support/mod.rs"]
#[allow(dead_code)]
mod support;

struct Dns;
impl Resolve for Dns {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        Box::pin(async move {
            let ip = if host == "api.mistral.test" {
                "93.184.215.14"
            } else {
                "127.0.0.1"
            };
            Ok(vec![SocketAddr::new(ip.parse().unwrap(), port)])
        })
    }
}

struct Pac {
    answer: Vec<String>,
    calls: AtomicUsize,
    seen: Mutex<Vec<String>>,
}
impl ProxyResolver for Pac {
    fn resolve(&self, url: &str) -> Result<Vec<String>, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(url.to_owned());
        Ok(self.answer.clone())
    }
}

fn url(host: &str, port: u16, secure: bool) -> url::Url {
    url::Url::parse(&format!(
        "{}://{host}:{port}/v1/audio/transcriptions/realtime?model=fixture",
        if secure { "wss" } else { "ws" }
    ))
    .unwrap()
}

async fn observed_proxy(pac: bool) -> (String, Result<(), ServiceStreamError>, usize, Vec<String>) {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = proxy.local_addr().unwrap();
    let resolver = Arc::new(Pac {
        answer: vec![format!("http://{address}")],
        calls: AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
    });
    let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
    let manual = Upstream::from_values(None, Some(&format!("http://{address}")), None, None);
    guard.set_upstream(if pac {
        Upstream::none().with_resolver(resolver.clone())
    } else {
        manual
    });
    let opened = tokio::spawn(async move {
        guard
            .open_service_stream(&url("api.mistral.test", 443, true))
            .await
            .map(|_| ())
    });
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(2), proxy.accept())
        .await
        .unwrap()
        .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(socket.read_u8().await.unwrap());
    }
    socket
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .unwrap();
    (
        String::from_utf8(head).unwrap(),
        opened.await.unwrap(),
        resolver.calls.load(Ordering::SeqCst),
        resolver.seen.lock().unwrap().clone(),
    )
}

#[tokio::test]
async fn selected_manual_proxy_receives_checked_connect() {
    let (head, result, calls, seen) = observed_proxy(false).await;
    assert!(
        head.starts_with("CONNECT 93.184.215.14:443 HTTP/1.1"),
        "{head}"
    );
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(calls, 0);
    assert_eq!(seen, Vec::<String>::new());
}

#[tokio::test]
async fn selected_pac_proxy_receives_checked_connect() {
    let (head, result, calls, seen) = observed_proxy(true).await;
    assert!(
        head.starts_with("CONNECT 93.184.215.14:443 HTTP/1.1"),
        "{head}"
    );
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(calls, 1);
    assert_eq!(
        seen,
        ["https://api.mistral.test/v1/audio/transcriptions/realtime?model=fixture"]
    );
}

#[tokio::test]
async fn dead_manual_proxy_never_opens_direct_socket() {
    for pac in [false, true] {
        let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = dead.local_addr().unwrap();
        drop(dead);
        let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
        let manual = Upstream::from_values(None, Some(&format!("http://{address}")), None, None);
        let resolver = Arc::new(Pac {
            answer: vec![format!("http://{address}")],
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        });
        guard.set_upstream(if pac {
            Upstream::none().with_resolver(resolver.clone())
        } else {
            manual
        });
        let result = guard
            .open_service_stream(&url(
                "local.test",
                direct.local_addr().unwrap().port(),
                true,
            ))
            .await;
        assert!(matches!(
            &result,
            Err(ServiceStreamError::Route(Error::ProxyUnreachable))
        ));
        assert_eq!(
            result.err().unwrap().to_string(),
            "The proxy could not be reached."
        );
        assert_eq!(resolver.calls.load(Ordering::SeqCst), usize::from(pac));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), direct.accept())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn pac_direct_and_unconfigured_routes_connect_directly() {
    for pac in [false, true] {
        let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = destination.local_addr().unwrap().port();
        let resolver = Arc::new(Pac {
            answer: vec!["direct://".into()],
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        });
        let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
        guard.set_upstream(if pac {
            Upstream::none().with_resolver(resolver.clone())
        } else {
            Upstream::none()
        });
        let mut stream = guard
            .open_service_stream(&url("local.test", port, false))
            .await
            .unwrap();
        let (mut server, _) = destination.accept().await.unwrap();
        stream.write_all(b"ping").await.unwrap();
        let mut bytes = [0; 4];
        server.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"ping");
        assert_eq!(resolver.calls.load(Ordering::SeqCst), usize::from(pac));
    }
}

#[tokio::test]
async fn socks_stream_uses_checked_numeric_target() {
    let proxy = support::Proxy::new(support::Kind::Socks5, false, false).await;
    let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
    guard.set_upstream(Upstream::from_values(None, Some(&proxy.url), None, None));
    let mut stream = guard
        .open_service_stream(&url("api.mistral.test", 443, true))
        .await
        .unwrap();
    stream.write_all(b"ping").await.unwrap();
    let mut echo = [0; 4];
    stream.read_exact(&mut echo).await.unwrap();
    assert_eq!(&echo, b"ping");
    assert_eq!(*proxy.targets.lock().unwrap(), ["93.184.215.14:443"]);
}

#[tokio::test]
async fn two_service_streams_use_the_selected_proxy_independently() {
    let proxy = support::Proxy::new(support::Kind::Socks5, false, false).await;
    let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
    guard.set_upstream(Upstream::from_values(None, Some(&proxy.url), None, None));
    let request = url("api.mistral.test", 443, true);
    let (first, second) = tokio::join!(
        guard.open_service_stream(&request),
        guard.open_service_stream(&request)
    );
    let mut first = first.unwrap();
    let mut second = second.unwrap();
    first.write_all(b"one").await.unwrap();
    second.write_all(b"two").await.unwrap();
    let mut one = [0; 3];
    let mut two = [0; 3];
    first.read_exact(&mut one).await.unwrap();
    second.read_exact(&mut two).await.unwrap();
    assert_eq!(&one, b"one");
    assert_eq!(&two, b"two");
    assert_eq!(
        *proxy.targets.lock().unwrap(),
        ["93.184.215.14:443", "93.184.215.14:443"]
    );
}

#[tokio::test]
async fn unsupported_service_scheme_is_refused_before_network() {
    let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
    let request = url::Url::parse("ftp://api.mistral.test/").unwrap();
    assert!(matches!(
        guard.open_service_stream(&request).await,
        Err(ServiceStreamError::Refused(_))
    ));
}
