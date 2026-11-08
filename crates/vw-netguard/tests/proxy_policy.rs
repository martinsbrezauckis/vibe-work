//! Offline guard tests through local HTTP, TLS and SOCKS proxies. No fixture opens Internet sockets.
//! Exercises refusals before proxy/PAC lookup, IP pinning, sign-in, ordered fallback and explicit Settings.
#![cfg(feature = "proxy-transports")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "support/client.rs"]
mod proxy_client;
mod support;

use std::net::SocketAddr;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use support::{Kind, Proxy, ca, head};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use vw_netguard::upstream::{Error, ProxyResolver, pac_routes};
use vw_netguard::{NetGuard, Resolve, ResolveFuture, Upstream};

struct Dns(AtomicUsize);
impl Resolve for Dns {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        let ips = match host {
            "allowed.test" => vec!["93.184.215.14"],
            "ipv6.test" => vec!["2606:4700:4700::1111"],
            "mixed.test" => vec!["93.184.215.14", "127.0.0.1"],
            "rebind.test" if self.0.fetch_add(1, Ordering::SeqCst) == 0 => vec!["93.184.215.14"],
            _ => vec!["10.0.0.1"],
        };
        Box::pin(async move {
            Ok(ips
                .into_iter()
                .map(|ip| SocketAddr::new(ip.parse().unwrap(), port))
                .collect())
        })
    }
}
fn guard(upstream: Upstream) -> (NetGuard, SocketAddr) {
    let guard = NetGuard::bind(Arc::new(Dns(AtomicUsize::new(0)))).unwrap();
    guard.set_upstream(upstream);
    guard.serve();
    proxy_client::addr(&guard.proxy_url());
    let address = proxy_client::addr(&guard.proxy_url());
    (guard, address)
}
fn via(proxy: &Proxy) -> Upstream {
    Upstream::from_values(None, None, Some(&proxy.url), None).with_certificates(vec![ca().to_vec()])
}
async fn connect(address: SocketAddr, host: &str) -> (TcpStream, String) {
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(&proxy_client::authorize(
            address,
            format!("CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\n\r\n").as_bytes(),
        ))
        .await
        .unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(3), head(&mut client))
        .await
        .unwrap();
    (client, answer)
}
async fn echo(address: SocketAddr, host: &str) {
    let (mut client, answer) = connect(address, host).await;
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    client.write_all(b"test tunnel bytes").await.unwrap();
    let mut bytes = [0; 17];
    tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"test tunnel bytes");
}

#[tokio::test]
async fn every_transport_preserves_policy_auth_and_rebinding_pins() {
    for kind in [Kind::Http, Kind::Https, Kind::Socks4, Kind::Socks5] {
        let proxy = Proxy::new(kind, true, false).await;
        let (_guard, address) = guard(via(&proxy));
        for host in [
            "denied.test",
            "mixed.test",
            "127.0.0.1",
            "169.254.169.254",
            "[::1]",
        ] {
            let (_, answer) = connect(address, host).await;
            assert!(
                answer.starts_with("HTTP/1.1 403"),
                "{kind:?}: {host}: {answer}"
            );
        }
        assert_eq!(
            proxy.count(),
            0,
            "{kind:?}: refusals must precede even the proxy TCP connection"
        );
        echo(address, "allowed.test").await;
        echo(address, "rebind.test").await;
        assert_eq!(
            *proxy.targets.lock().unwrap(),
            ["93.184.215.14:443", "93.184.215.14:443"]
        );
        let (_, answer) = connect(address, "rebind.test").await;
        assert!(answer.starts_with("HTTP/1.1 403"));
        assert_eq!(
            proxy.count(),
            2,
            "rebinding cannot open a third proxy connection"
        );
    }
}

#[tokio::test]
async fn proxy_tls_requires_trust_and_the_correct_name() {
    let proxy = Proxy::new(Kind::Https, false, false).await;
    let (_guard, address) = guard(Upstream::from_values(None, None, Some(&proxy.url), None));
    let (mut client, answer) = connect(address, "allowed.test").await;
    assert!(answer.starts_with("HTTP/1.1 502"));
    let mut body = String::new();
    client.read_to_string(&mut body).await.unwrap();
    assert!(body.contains("The proxy certificate could not be verified."));
    assert!(
        proxy.targets.lock().unwrap().is_empty(),
        "no CONNECT before TLS verification"
    );
    let wrong = Proxy::wrong_name().await;
    let (_guard, address) = guard(
        Upstream::from_values(None, None, Some(&wrong.url), None)
            .with_certificates(vec![ca().to_vec()]),
    );
    let (_, answer) = connect(address, "allowed.test").await;
    assert!(
        answer.starts_with("HTTP/1.1 502"),
        "a trusted certificate for the wrong name must fail"
    );
    assert!(wrong.targets.lock().unwrap().is_empty());
    // localhost is a second valid certificate identity, resolved only for the proxy itself.
    let proxy_url = format!("https://localhost:{}", proxy.address.port());
    let (_guard, address) = guard(
        Upstream::from_values(None, None, Some(&proxy_url), None)
            .with_certificates(vec![ca().to_vec()]),
    );
    echo(address, "allowed.test").await;
}

#[cfg(target_os = "linux")]
#[test]
fn proxy_tls_keeps_platform_roots_when_company_roots_are_added() {
    // A child process isolates rustls-native-certs' store override from every other test.
    const CHILD: &str = "VW_PROXY_PLATFORM_TRUST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                // The child generates its own CA; write its PEM before the platform verifier loads it.
                std::fs::write(
                    std::env::var_os("SSL_CERT_FILE").unwrap(),
                    support::ca_pem(),
                )
                .unwrap();
                let proxy = Proxy::new(Kind::Https, false, false).await;
                for roots in [vec![], vec![include_bytes!("fixtures/ca.der").to_vec()]] {
                    let (_guard, address) = guard(
                        Upstream::from_values(None, None, Some(&proxy.url), None)
                            .with_certificates(roots),
                    );
                    echo(address, "allowed.test").await;
                }
            });
        return;
    }
    let root =
        std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let fixtures = root.join(format!("proxy-platform-trust-{}", std::process::id()));
    std::fs::create_dir_all(&fixtures).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "proxy_tls_keeps_platform_roots_when_company_roots_are_added",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SSL_CERT_FILE", fixtures.join("ca.pem"))
        .env("SSL_CERT_DIR", &fixtures)
        .output()
        .unwrap();
    std::fs::remove_dir_all(fixtures).unwrap();
    assert!(
        output.status.success(),
        "platform trust child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn malformed_company_certificate_refuses_tls_before_connect() {
    let proxy = Proxy::new(Kind::Https, false, false).await;
    let (_guard, address) = guard(
        Upstream::from_values(None, None, Some(&proxy.url), None)
            .with_certificates(vec![ca().to_vec(), b"not a certificate".to_vec()]),
    );
    let (_, answer) = connect(address, "allowed.test").await;
    assert!(answer.starts_with("HTTP/1.1 502"));
    assert!(proxy.targets.lock().unwrap().is_empty());
}

#[tokio::test]
async fn socks5_keeps_ipv6_and_plain_http_uses_origin_form_without_proxy_auth() {
    for kind in [Kind::Socks5, Kind::Https] {
        let proxy = Proxy::new(kind, true, false).await;
        let (_guard, address) = guard(via(&proxy));
        echo(address, "ipv6.test").await;
        assert_eq!(
            proxy.targets.lock().unwrap()[0],
            "[2606:4700:4700::1111]:443"
        );
        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(&proxy_client::authorize(
                address,
                b"GET http://allowed.test/path?x=1 HTTP/1.1\r\nHost: allowed.test\r\n\r\n",
            ))
            .await
            .unwrap();
        let mut answer = String::new();
        tokio::time::timeout(Duration::from_secs(2), client.read_to_string(&mut answer))
            .await
            .unwrap()
            .unwrap();
        assert!(answer.starts_with("HTTP/1.1 200"));
        assert!(answer.contains("GET /path?x=1 HTTP/1.1"));
        assert!(answer.contains("Host: allowed.test"));
        assert!(!answer.contains("stolen") && !answer.contains("dXNlcjpzZWNyZXQ="));
    }
}

struct Stub {
    routes: Vec<String>,
    queries: Mutex<Vec<String>>,
}
impl ProxyResolver for Stub {
    fn resolve(&self, url: &str) -> Result<Vec<String>, Error> {
        self.queries.lock().unwrap().push(url.to_owned());
        Ok(self.routes.clone())
    }
}
fn stub(routes: Vec<String>) -> Arc<Stub> {
    Arc::new(Stub {
        routes,
        queries: Mutex::new(Vec::new()),
    })
}

#[tokio::test]
async fn pac_fallbacks_preserve_order_and_never_receive_refused_names() {
    let first = Proxy::new(Kind::Http, false, true).await;
    let second = Proxy::new(Kind::Socks5, false, false).await;
    let last = Proxy::new(Kind::Https, false, false).await;
    let resolver = stub(
        pac_routes(&format!(
            "PROXY {}; SOCKS5 {}; HTTPS {}; DIRECT",
            first.address, second.address, last.address
        ))
        .unwrap(),
    );
    let (_guard, address) = guard(Upstream::none().with_resolver(resolver.clone()));
    let (_, answer) = connect(address, "denied.test").await;
    assert!(answer.starts_with("HTTP/1.1 403"));
    assert!(resolver.queries.lock().unwrap().is_empty());
    echo(address, "allowed.test").await;
    assert_eq!((first.count(), second.count(), last.count()), (1, 1, 0));
    assert_eq!(*resolver.queries.lock().unwrap(), ["https://allowed.test/"]);
}

#[tokio::test]
async fn settings_win_over_pac_and_errors_never_imply_direct() {
    let proxy = Proxy::new(Kind::Socks5, false, false).await;
    let resolver = stub(vec!["direct://".into()]);
    let (_guard, address) = guard(via(&proxy).with_resolver(resolver.clone()));
    echo(address, "allowed.test").await;
    assert!(resolver.queries.lock().unwrap().is_empty());
    for routes in [
        vec![],
        vec!["invalid://secret@proxy".into()],
        vec!["http://bad/path".into()],
    ] {
        let (_guard, address) = guard(Upstream::none().with_resolver(stub(routes)));
        let (_, answer) = connect(address, "allowed.test").await;
        assert!(answer.starts_with("HTTP/1.1 502"));
    }
    for malformed in ["", "UNKNOWN proxy:1; DIRECT", "SOCKS5", "DIRECT proxy:1"] {
        assert!(pac_routes(malformed).is_err());
    }
}

#[tokio::test]
async fn proxy_failures_have_short_states_and_actionable_recorded_outcomes() {
    for kind in [Kind::Https, Kind::Socks5] {
        let proxy = Proxy::new(kind, true, true).await;
        let (guard, address) = guard(via(&proxy));
        let (mut client, answer) = connect(address, "allowed.test").await;
        assert!(answer.starts_with("HTTP/1.1 502"));
        let mut body = String::new();
        client.read_to_string(&mut body).await.unwrap();
        assert!(
            body.contains("The proxy refused the connection.")
                || body.contains("The proxy sign-in was refused.")
        );
        assert!(!body.contains("secret") && !body.contains("user"));
        let debug = format!("{:?}", via(&proxy));
        assert!(!debug.contains("secret") && !debug.contains("user"));
        // Record inspection below is intentionally through the same public log the core consumes.
        let refusals = guard.refusals_since(0);
        assert_eq!(refusals.len(), 1);
        assert!(refusals[0].reason.contains("Ask the person"));
        assert!(!refusals[0].reason.contains("secret"));
    }
}

/// Configure the OS PAC URL to <http://127.0.0.1:19070/proxy.pac> in a disposable OS user first.
/// The test supplies the PAC and all proxy listeners; no external server or real DNS answer is used.
#[tokio::test]
#[ignore = "requires the native OS PAC URL configured to the local fixture"]
async fn native_pac_uses_ordered_transports() {
    use tokio::net::TcpListener;
    let first = Proxy::new(Kind::Http, false, true).await;
    // Native Windows PAC's SOCKS type is SOCKS4; CFNetwork/libproxy's generic SOCKS type is SOCKS5.
    let kind = if cfg!(windows) {
        Kind::Socks4
    } else {
        Kind::Socks5
    };
    let second = Proxy::new(kind, false, false).await;
    let listener = TcpListener::bind("127.0.0.1:19070").await.unwrap();
    let pac = format!(
        "function FindProxyForURL(url, host) {{ return 'PROXY {}; SOCKS {}; DIRECT'; }}",
        first.address, second.address
    );
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = head(&mut socket).await;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{pac}",
                pac.len()
            );
            let _ = socket.write_all(reply.as_bytes()).await;
        }
    });
    let (_guard, address) = guard(Upstream::system().with_certificates(vec![ca().to_vec()]));
    let (_, denied) = connect(address, "denied.test").await;
    assert!(denied.starts_with("HTTP/1.1 403"));
    echo(address, "allowed.test").await;
    assert_eq!((first.count(), second.count()), (1, 1));
    assert_eq!(*second.targets.lock().unwrap(), ["93.184.215.14:443"]);
    server.abort();
}
