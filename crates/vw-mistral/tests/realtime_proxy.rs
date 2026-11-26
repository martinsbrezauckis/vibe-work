//! Offline realtime connection tests through netguard's checked service stream.
#![cfg(all(feature = "proxy-transports", target_os = "linux"))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use futures_util::{SinkExt as _, StreamExt as _};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use vw_mistral::realtime;
use vw_netguard::upstream::{Error, ProxyResolver};
use vw_netguard::{NetGuard, Resolve, ResolveFuture, Upstream};

struct Dns(IpAddr);
impl Resolve for Dns {
    fn resolve<'a>(&'a self, _: &'a str, port: u16) -> ResolveFuture<'a> {
        let address = SocketAddr::new(self.0, port);
        Box::pin(async move { Ok(vec![address]) })
    }
}

struct Pac {
    answer: Vec<String>,
    calls: AtomicUsize,
}
impl ProxyResolver for Pac {
    fn resolve(&self, _: &str) -> Result<Vec<String>, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.answer.clone())
    }
}

fn acceptor() -> (tokio_rustls::TlsAcceptor, String) {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec!["api.mistral.test".to_owned(), "127.0.0.1".to_owned()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(leaf.der().to_vec())],
        PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
    )
    .unwrap();
    (tokio_rustls::TlsAcceptor::from(Arc::new(config)), ca.pem())
}

async fn fixture(acceptor: tokio_rustls::TlsAcceptor) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = acceptor.accept(tcp).await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tls).await.unwrap();
        ws.send(Message::Text(r#"{"type":"session.created"}"#.into()))
            .await
            .unwrap();
        let update = ws.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(update.contains("session.update"), "{update}");
        ws.send(Message::Text(
            r#"{"type":"transcription.text.delta","text":"ready"}"#.into(),
        ))
        .await
        .unwrap();
    });
    (addr, task)
}

async fn proxy(
    to: SocketAddr,
) -> (
    SocketAddr,
    tokio::sync::oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut client, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(client.read_u8().await.unwrap());
        }
        sender.send(String::from_utf8(head).unwrap()).unwrap();
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        let mut server = TcpStream::connect(to).await.unwrap();
        let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
    });
    (addr, receiver, task)
}

fn guard(ip: IpAddr, upstream: Upstream) -> NetGuard {
    let guard = NetGuard::bind_system_client(Arc::new(Dns(ip))).unwrap();
    guard.set_upstream(upstream);
    guard
}

async fn child(
    base: &str,
    mode: &str,
    target: IpAddr,
    proxy: &str,
    ca_file: &std::path::Path,
    expected: &str,
) {
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tls_child", "--nocapture"])
        .env("SSL_CERT_FILE", ca_file)
        .env("VW_WS_CHILD_BASE", base)
        .env("VW_WS_CHILD_MODE", mode)
        .env("VW_WS_CHILD_TARGET", target.to_string())
        .env("VW_WS_CHILD_PROXY", proxy)
        .env("VW_WS_CHILD_EXPECTED", expected)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "child failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn tls_child() {
    let Ok(base) = std::env::var("VW_WS_CHILD_BASE") else {
        return;
    };
    let mode = std::env::var("VW_WS_CHILD_MODE").unwrap();
    let target: IpAddr = std::env::var("VW_WS_CHILD_TARGET")
        .unwrap()
        .parse()
        .unwrap();
    let endpoint = std::env::var("VW_WS_CHILD_PROXY").unwrap();
    let expected = std::env::var("VW_WS_CHILD_EXPECTED").unwrap();
    let result = if mode == "public" {
        vw_mistral::network::set_current(vw_mistral::network::NetworkSettings::from_lookup(
            |key| (key == "HTTPS_PROXY").then(|| endpoint.clone()),
        ));
        realtime::connect(&base, "fixture-key").await
    } else {
        let resolver = Arc::new(Pac {
            answer: vec![endpoint.clone()],
            calls: AtomicUsize::new(0),
        });
        let upstream = match mode.as_str() {
            "manual" => Upstream::from_values(None, Some(&endpoint), None, None),
            "pac" | "pac-direct" => Upstream::none().with_resolver(resolver.clone()),
            "none" => Upstream::none(),
            _ => panic!("unknown child mode"),
        };
        let guard = guard(target, upstream);
        let result = realtime::connect_via(&base, "fixture-key", &guard).await;
        assert_eq!(
            resolver.calls.load(Ordering::SeqCst),
            usize::from(matches!(mode.as_str(), "pac" | "pac-direct"))
        );
        result
    };
    match expected.as_str() {
        "success" => {
            let (sender, mut receiver) = result.expect("realtime did not connect");
            assert_eq!(
                receiver.next().await,
                realtime::RealtimeEvent::Delta("ready".to_owned())
            );
            sender.close().await;
        }
        "tls-fail" => assert_eq!(
            result.err().unwrap().to_string(),
            "Could not reach Mistral: the secure connection failed."
        ),
        "proxy-fail" => assert_eq!(
            result.err().unwrap().to_string(),
            "Vibe Work could not reach the proxy."
        ),
        _ => panic!("unknown expected outcome"),
    }
}

#[tokio::test]
async fn realtime_uses_checked_manual_and_pac_proxies_and_keeps_tls_name() {
    let (tls, ca) = acceptor();
    let root = tempfile::tempdir().unwrap();
    let ca_file = root.path().join("ca.pem");
    std::fs::write(&ca_file, ca).unwrap();
    let checked: IpAddr = "93.184.215.14".parse().unwrap();

    for pac in [false, true] {
        let (server, done) = fixture(tls.clone()).await;
        let (proxy_addr, seen, proxy_task) = proxy(server).await;
        let endpoint = format!("http://{proxy_addr}");
        child(
            "https://api.mistral.test",
            if pac { "pac" } else { "manual" },
            checked,
            &endpoint,
            &ca_file,
            "success",
        )
        .await;
        done.await.unwrap();
        let head = seen.await.unwrap();
        assert!(
            head.starts_with("CONNECT 93.184.215.14:443 HTTP/1.1"),
            "{head}"
        );
        proxy_task.abort();
    }

    // The checked IP is never used as the TLS identity; a different requested name fails.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = listener.local_addr().unwrap();
    let wrong_tls = tls.clone();
    let wrong_task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let _ = wrong_tls.accept(tcp).await;
    });
    let (proxy_addr, seen, proxy_task) = proxy(server).await;
    child(
        "https://wrong.test",
        "manual",
        checked,
        &format!("http://{proxy_addr}"),
        &ca_file,
        "tls-fail",
    )
    .await;
    assert!(
        seen.await
            .unwrap()
            .starts_with("CONNECT 93.184.215.14:443 HTTP/1.1")
    );
    wrong_task.await.unwrap();
    proxy_task.abort();

    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead.local_addr().unwrap();
    drop(dead);
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = direct.local_addr().unwrap().port();
    child(
        &format!("https://api.mistral.test:{port}"),
        "manual",
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        &format!("http://{dead_addr}"),
        &ca_file,
        "proxy-fail",
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), direct.accept())
            .await
            .is_err()
    );

    for pac in [false, true] {
        let (server, done) = fixture(tls.clone()).await;
        child(
            &format!("https://api.mistral.test:{}", server.port()),
            if pac { "pac-direct" } else { "none" },
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            "direct://",
            &ca_file,
            "success",
        )
        .await;
        done.await.unwrap();
    }

    // The crate's public entry point also applies current network settings in a proxy-enabled build.
    let (server, done) = fixture(tls).await;
    let (proxy_addr, seen, proxy_task) = proxy(server).await;
    child(
        &format!("https://127.0.0.1:{}", server.port()),
        "public",
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        &format!("http://{proxy_addr}"),
        &ca_file,
        "success",
    )
    .await;
    done.await.unwrap();
    assert!(
        seen.await
            .unwrap()
            .starts_with(&format!("CONNECT 127.0.0.1:{} HTTP/1.1", server.port()))
    );
    proxy_task.abort();
}
