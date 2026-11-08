//! Listener credentials over real sockets. Cargo runs this with both transport configurations.
//! Scripted DNS and a local upstream prove refusals happen before any outbound work.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use base64::Engine as _;
use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use vw_netguard::{NetGuard, Resolve, ResolveFuture, Upstream};

struct Dns(AtomicUsize);
impl Resolve for Dns {
    fn resolve<'a>(&'a self, _: &'a str, port: u16) -> ResolveFuture<'a> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(vec![SocketAddr::new(
                "93.184.215.14".parse().unwrap(),
                port,
            )])
        })
    }
}

fn auth(proxy: &str) -> String {
    let url = url::Url::parse(proxy).unwrap();
    format!(
        "Proxy-Authorization: Basic {}\r\n",
        base64::engine::general_purpose::STANDARD.encode(format!(
            "{}:{}",
            url.username(),
            url.password().unwrap_or_default()
        ))
    )
}

async fn request(proxy: &str, line: &str, headers: &str) -> String {
    let url = url::Url::parse(proxy).unwrap();
    let mut client = TcpStream::connect(("127.0.0.1", url.port().unwrap()))
        .await
        .unwrap();
    client
        .write_all(format!("{line} HTTP/1.1\r\nHost: public.test\r\n{headers}\r\n").as_bytes())
        .await
        .unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !response.ends_with(b"\r\n\r\n") {
            response.push(client.read_u8().await.unwrap());
        }
    })
    .await
    .unwrap();
    String::from_utf8(response).unwrap()
}

async fn refuses_without_work(lines: &[&str]) {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dns = Arc::new(Dns(AtomicUsize::new(0)));
    let guard = NetGuard::bind(dns.clone()).unwrap();
    guard.set_upstream(Upstream::from_values(
        None,
        None,
        Some(&format!(
            "http://user:secret@{}",
            upstream.local_addr().unwrap()
        )),
        None,
    ));
    guard.serve();
    let proxy = guard.proxy_url();
    let dials = Arc::new(AtomicUsize::new(0));
    let count = dials.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = upstream.accept().await.unwrap();
            count.fetch_add(1, Ordering::SeqCst);
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(stream.read_u8().await.unwrap());
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        }
    });
    for line in lines {
        for headers in [
            String::new(),
            "Proxy-Authorization: Basic wrong\r\n".into(),
            auth(&proxy).repeat(2),
        ] {
            let response = request(&proxy, line, &headers).await;
            assert!(
                response.starts_with("HTTP/1.1 407"),
                "{line}: {response}; upstream dials: {}",
                dials.load(Ordering::SeqCst)
            );
            assert!(response.contains("Proxy-Authenticate: Basic"));
        }
    }
    assert_eq!(dns.0.load(Ordering::SeqCst), 0);
    assert_eq!(dials.load(Ordering::SeqCst), 0);
    server.abort();
    assert!(!guard.wait_seen("probe", Duration::ZERO).await);
    assert!(!guard.wait_seen_direct("probe", Duration::ZERO).await);
    guard.shutdown();
}

#[tokio::test]
async fn unauthenticated_connect_never_resolves_or_dials() {
    refuses_without_work(&["CONNECT public.test:443"]).await;
}

#[tokio::test]
async fn unauthenticated_absolute_form_never_resolves_or_dials() {
    refuses_without_work(&["GET http://public.test/path"]).await;
}

#[tokio::test]
async fn authentication_precedes_canary_logic() {
    refuses_without_work(&[
        "GET http://probe.vw-guard.invalid/",
        "GET /__vw_guard/probe",
    ])
    .await;
}

#[tokio::test]
async fn credentials_are_per_listener_and_rotate_with_the_store() {
    let guard = NetGuard::bind(Arc::new(Dns(AtomicUsize::new(0)))).unwrap();
    guard.serve();
    let a = guard.proxy_url_for_store("a", "a").unwrap();
    let b = guard.proxy_url_for_store("b", "b").unwrap();
    let line = "GET http://probe.vw-guard.invalid/";
    for other in [&b, &guard.proxy_url()] {
        assert!(
            request(&a, line, &auth(other))
                .await
                .starts_with("HTTP/1.1 407")
        );
    }
    assert!(
        request(&a, line, &auth(&a))
            .await
            .starts_with("HTTP/1.1 200")
    );
    guard.forget_store("a");
    let replacement = guard.proxy_url_for_store("a", "a").unwrap();
    assert_ne!(auth(&a), auth(&replacement));
    assert!(
        request(&replacement, line, &auth(&a))
            .await
            .starts_with("HTTP/1.1 407")
    );
    assert!(
        request(&replacement, line, &auth(&replacement))
            .await
            .starts_with("HTTP/1.1 200")
    );
    guard.shutdown();
}

#[tokio::test]
async fn authenticated_connect_and_absolute_requests_use_only_upstream_credentials() {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let guard = NetGuard::bind(Arc::new(Dns(AtomicUsize::new(0)))).unwrap();
    guard.set_upstream(Upstream::from_values(
        None,
        None,
        Some(&format!(
            "http://user:secret@{}",
            upstream.local_addr().unwrap()
        )),
        None,
    ));
    guard.serve();
    let proxy = guard.proxy_url();
    let ingress = auth(&proxy);
    let forbidden = ingress.clone();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(socket.read_u8().await.unwrap());
            }
            let head = String::from_utf8(head).unwrap();
            assert!(head.contains("Proxy-Authorization: Basic dXNlcjpzZWNyZXQ=\r\n"));
            assert!(!head.contains(&forbidden));
            assert!(head.contains("93.184.215.14"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        }
    });
    for line in ["CONNECT public.test:443", "GET http://public.test/path"] {
        assert!(
            request(&proxy, line, &ingress)
                .await
                .starts_with("HTTP/1.1 200")
        );
    }
    server.await.unwrap();
    guard.shutdown();
}

#[tokio::test]
async fn locked_listeners_still_challenge_before_reporting_vault_state() {
    let guard = NetGuard::bind(Arc::new(Dns(AtomicUsize::new(0)))).unwrap();
    guard.set_enabled(false);
    guard.serve();
    let proxy = guard.proxy_url();
    assert!(
        request(&proxy, "CONNECT public.test:443", "")
            .await
            .starts_with("HTTP/1.1 407")
    );
    assert!(
        request(&proxy, "CONNECT public.test:443", &auth(&proxy))
            .await
            .starts_with("HTTP/1.1 503")
    );
    guard.shutdown();
}

#[test]
fn credentials_never_enter_canary_pages_or_browser_process_arguments() {
    let guard = NetGuard::bind(Arc::new(Dns(AtomicUsize::new(0)))).unwrap();
    let proxy = guard.proxy_url();
    let parsed = url::Url::parse(&proxy).unwrap();
    let secret = parsed.password().unwrap();
    assert_eq!(secret.len(), 64);
    let canary = NetGuard::loopback_canary_url(&proxy, "test");
    let arguments = vw_netguard::chromium::webview2_arguments(&proxy).unwrap();
    assert!(!canary.contains(secret));
    assert!(!arguments.contains(secret));
    assert_eq!(url::Url::parse(&canary).unwrap().username(), "");
    assert!(!format!("{guard:?}").contains(secret));
    guard.shutdown();
}

#[tokio::test]
async fn stopping_a_store_or_guard_closes_accepted_tunnels_and_pending_heads() {
    for stop in ["store", "prefix", "guard"] {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let guard = NetGuard::bind(Arc::new(Dns(AtomicUsize::new(0)))).unwrap();
        guard.set_upstream(Upstream::from_values(
            None,
            None,
            Some(&format!("http://{}", upstream.local_addr().unwrap())),
            None,
        ));
        let proxy = guard.proxy_url_for_store("folder/a", "a").unwrap();
        let port = url::Url::parse(&proxy).unwrap().port().unwrap();
        let mut pending = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        pending.write_all(b"CONNECT ").await.unwrap();
        let mut tunnel = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        tunnel
            .write_all(
                format!("CONNECT public.test:443 HTTP/1.1\r\n{}\r\n", auth(&proxy)).as_bytes(),
            )
            .await
            .unwrap();
        let (mut remote, _) = upstream.accept().await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(remote.read_u8().await.unwrap());
        }
        remote
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        head.clear();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(tunnel.read_u8().await.unwrap());
        }
        match stop {
            "store" => guard.forget_store("folder/a"),
            "prefix" => guard.forget_stores_under("folder/"),
            _ => guard.shutdown(),
        }
        let replacement = guard
            .proxy_url_for_store("folder/a", "replacement")
            .unwrap();
        assert_ne!(auth(&proxy), auth(&replacement));
        for socket in [&mut pending, &mut tunnel, &mut remote] {
            assert!(
                tokio::time::timeout(Duration::from_secs(1), socket.read_u8())
                    .await
                    .unwrap()
                    .is_err(),
                "{stop}: accepted connection outlived its listener"
            );
        }
        guard.shutdown();
    }
}
