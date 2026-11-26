//! Service automatic routes preserve checked local destinations; browser and manual relay policy stays strict.
//! All endpoints are local fixtures. No desktop settings, PAC download, or Internet sockets are used.
#![cfg(feature = "proxy-transports")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "support/client.rs"]
mod proxy_client;

use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use vw_netguard::upstream::{Error, ProxyResolver};
use vw_netguard::{NetGuard, Resolve, ResolveFuture, Upstream};

struct Dns;
impl Resolve for Dns {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        let ip = if host == "lan.test" {
            "10.20.30.40"
        } else {
            "127.0.0.1"
        };
        Box::pin(async move { Ok(vec![SocketAddr::new(ip.parse().unwrap(), port)]) })
    }
}

struct GatedDns {
    entered: Notify,
    release: Notify,
    calls: AtomicUsize,
}
impl Resolve for GatedDns {
    fn resolve<'a>(&'a self, _: &'a str, port: u16) -> ResolveFuture<'a> {
        Box::pin(async move {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(vec![SocketAddr::new("10.20.30.40".parse().unwrap(), port)])
        })
    }
}

struct Native {
    answer: Result<Vec<String>, Error>,
    calls: AtomicUsize,
}
impl ProxyResolver for Native {
    fn resolve(&self, _: &str) -> Result<Vec<String>, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer.clone()
    }
}

async fn request(guard: &NetGuard, target: &str) -> String {
    let address = proxy_client::addr(&guard.proxy_url());
    let mut client = TcpStream::connect(address).await.unwrap();
    let head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    client
        .write_all(&proxy_client::authorize(address, head.as_bytes()))
        .await
        .unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(4), async {
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(client.read_u8().await.unwrap());
        }
    })
    .await
    .unwrap();
    String::from_utf8(bytes).unwrap()
}

#[tokio::test]
async fn automatic_direct_reaches_checked_loopback_while_browser_and_manual_still_refuse() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = destination.local_addr().unwrap().port();
    let native = Arc::new(Native {
        answer: Ok(vec!["direct://".into()]),
        calls: AtomicUsize::new(0),
    });
    let upstream = || Upstream::from_values(None, None, None, None).with_resolver(native.clone());

    let automatic = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
    automatic.set_upstream(upstream());
    automatic.serve();
    let target = format!("127.0.0.1:{port}");
    let response = request(&automatic, &target).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let _accepted = tokio::time::timeout(Duration::from_secs(2), destination.accept())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        native.calls.load(Ordering::SeqCst),
        1,
        "local DIRECT must come from PAC"
    );

    let browser = NetGuard::bind(Arc::new(Dns)).unwrap();
    browser.set_upstream(upstream());
    browser.serve();
    assert!(request(&browser, &target).await.starts_with("HTTP/1.1 403"));

    automatic.set_upstream(
        Upstream::from_values(None, Some("http://127.0.0.1:9"), None, None)
            .with_resolver(native.clone()),
    );
    assert!(
        request(&automatic, &target)
            .await
            .starts_with("HTTP/1.1 403")
    );
}

#[tokio::test]
async fn automatic_private_destination_uses_pac_proxy_without_direct_lan_socket() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", proxy.local_addr().unwrap());
    let native = Arc::new(Native {
        answer: Ok(vec![endpoint]),
        calls: AtomicUsize::new(0),
    });
    let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
    guard.set_upstream(Upstream::from_values(None, None, None, None).with_resolver(native.clone()));
    guard.serve();
    let request_task = tokio::spawn(async move { request(&guard, "lan.test:443").await });
    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(3), proxy.accept())
        .await
        .unwrap()
        .unwrap();
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(stream.read_u8().await.unwrap());
    }
    assert!(String::from_utf8_lossy(&bytes).starts_with("CONNECT 10.20.30.40:443 HTTP/1.1"));
    stream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .unwrap();
    assert!(request_task.await.unwrap().starts_with("HTTP/1.1 200"));
    assert_eq!(native.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn automatic_pac_error_for_local_target_never_falls_back_to_direct() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = destination.local_addr().unwrap().port();
    let native = Arc::new(Native {
        answer: Err(Error::Discovery),
        calls: AtomicUsize::new(0),
    });
    let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
    guard.set_upstream(Upstream::from_values(None, None, None, None).with_resolver(native.clone()));
    guard.serve();
    let response = request(&guard, &format!("127.0.0.1:{port}")).await;
    assert!(response.starts_with("HTTP/1.1 502"), "{response}");
    assert!(guard.proxy_lookup_failed_since(0));
    assert!(
        guard.refusals_since(0)[0]
            .reason
            .contains("OS proxy/PAC resolver failed")
    );
    assert_eq!(native.calls.load(Ordering::SeqCst), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), destination.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn model_relay_refuses_dead_pac_proxy_without_opening_a_direct_socket() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = destination.local_addr().unwrap().port();
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = unavailable.local_addr().unwrap();
    drop(unavailable);
    let native = Arc::new(Native {
        answer: Ok(vec![format!("http://{proxy}")]),
        calls: AtomicUsize::new(0),
    });
    let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
    guard.set_upstream(Upstream::none().with_resolver(native.clone()));
    guard.serve();
    let target = format!("api.mistral.ai:{port}");
    let (first, second) = tokio::join!(request(&guard, &target), request(&guard, &target));
    assert!(first.starts_with("HTTP/1.1 502"), "{first}");
    assert!(second.starts_with("HTTP/1.1 502"), "{second}");
    assert_eq!(native.calls.load(Ordering::SeqCst), 2);
    assert_eq!(guard.last_route(), None);
    assert!(guard.refusals_since(0).iter().all(|refusal| {
        refusal
            .reason
            .contains("selected upstream proxy could not be reached")
    }));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), destination.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn automatic_pac_dead_proxy_records_upstream_failure_without_direct_fallback() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = destination.local_addr().unwrap().port();
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_address = dead.local_addr().unwrap();
    drop(dead);
    let native = Arc::new(Native {
        answer: Ok(vec![format!("http://{dead_address}")]),
        calls: AtomicUsize::new(0),
    });
    let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
    guard.set_upstream(Upstream::from_values(None, None, None, None).with_resolver(native.clone()));
    guard.serve();

    let response = request(&guard, &format!("127.0.0.1:{port}")).await;
    assert!(response.starts_with("HTTP/1.1 502"), "{response}");
    assert!(guard.proxy_connect_failed_since(0));
    assert_eq!(guard.refusals_since(0).len(), 1);
    assert_eq!(
        guard.refusals_since(0)[0].reason,
        Error::ProxyUnreachable.outcome()
    );
    assert!(!guard.proxy_lookup_failed_since(0));
    assert_eq!(native.calls.load(Ordering::SeqCst), 1);
    assert_eq!(guard.last_route(), None);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), destination.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn direct_failure_does_not_claim_that_a_proxy_was_unreachable() {
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let native = Arc::new(Native {
        answer: Ok(vec!["direct://".into()]),
        calls: AtomicUsize::new(0),
    });
    let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
    guard.set_upstream(Upstream::from_values(None, None, None, None).with_resolver(native));
    guard.serve();

    let response = request(&guard, &format!("127.0.0.1:{port}")).await;
    assert!(response.starts_with("HTTP/1.1 502"), "{response}");
    assert!(!guard.proxy_connect_failed_since(0));
    assert_eq!(
        guard.refusals_since(0)[0].reason,
        Error::Unreachable.outcome()
    );
}

#[tokio::test]
async fn answered_503_is_a_proxy_refusal_not_an_unreachable_proxy() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let native = Arc::new(Native {
        answer: Ok(vec![format!("http://{}", proxy.local_addr().unwrap())]),
        calls: AtomicUsize::new(0),
    });
    let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
    guard.set_upstream(Upstream::from_values(None, None, None, None).with_resolver(native));
    guard.serve();
    let client = guard.clone();
    let request_task = tokio::spawn(async move { request(&client, "lan.test:443").await });
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), proxy.accept())
        .await
        .unwrap()
        .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(socket.read_u8().await.unwrap());
    }
    assert!(String::from_utf8_lossy(&head).starts_with("CONNECT 10.20.30.40:443 HTTP/1.1"));
    socket
        .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
        .await
        .unwrap();
    let response = request_task.await.unwrap();
    assert!(response.starts_with("HTTP/1.1 502"), "{response}");
    assert!(!guard.proxy_connect_failed_since(0));
    assert_eq!(guard.refusals_since(0)[0].reason, Error::Refused.outcome());
}

#[tokio::test]
async fn concurrent_guards_keep_lookup_failure_on_its_own_test() {
    let bind = |error| {
        let native = Arc::new(Native {
            answer: Err(error),
            calls: AtomicUsize::new(0),
        });
        let guard = NetGuard::bind_system_client(Arc::new(Dns)).unwrap();
        guard.set_upstream(Upstream::from_values(None, None, None, None).with_resolver(native));
        guard.serve();
        guard
    };
    let lookup = bind(Error::Discovery);
    let refusal = bind(Error::Refused);
    let (lookup_answer, refused_answer) = tokio::join!(
        request(&lookup, "lan.test:443"),
        request(&refusal, "lan.test:443")
    );
    assert!(lookup_answer.starts_with("HTTP/1.1 502"));
    assert!(refused_answer.starts_with("HTTP/1.1 502"));
    assert!(lookup.proxy_lookup_failed_since(0));
    assert!(!refusal.proxy_lookup_failed_since(0));
    assert!(!lookup.proxy_lookup_failed_since(lookup.last_seq()));
}

#[tokio::test]
async fn concurrent_reconfiguration_keeps_one_upstream_snapshot_per_request() {
    let proxy_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let native = Arc::new(Native {
        answer: Ok(vec![format!("http://{}", proxy_a.local_addr().unwrap())]),
        calls: AtomicUsize::new(0),
    });
    let dns = Arc::new(GatedDns {
        entered: Notify::new(),
        release: Notify::new(),
        calls: AtomicUsize::new(0),
    });
    let guard = NetGuard::bind_system_client(dns.clone()).unwrap();
    guard.set_upstream(Upstream::from_values(None, None, None, None).with_resolver(native.clone()));
    guard.serve();

    let first = guard.clone();
    let first_request = tokio::spawn(async move { request(&first, "lan.test:443").await });
    tokio::time::timeout(Duration::from_secs(3), dns.entered.notified())
        .await
        .expect("first request paused after route snapshot");

    guard.set_upstream(Upstream::from_values(
        None,
        Some(&format!("http://{}", proxy_b.local_addr().unwrap())),
        None,
        None,
    ));
    let second = request(&guard, "lan.test:443").await;
    assert!(second.starts_with("HTTP/1.1 403"), "{second}");
    dns.release.notify_one();

    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(3), proxy_a.accept())
        .await
        .unwrap()
        .unwrap();
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(stream.read_u8().await.unwrap());
    }
    assert!(String::from_utf8_lossy(&bytes).starts_with("CONNECT 10.20.30.40:443 HTTP/1.1"));
    stream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .unwrap();
    assert!(first_request.await.unwrap().starts_with("HTTP/1.1 200"));
    assert_eq!(native.calls.load(Ordering::SeqCst), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), proxy_b.accept())
            .await
            .is_err()
    );
}
