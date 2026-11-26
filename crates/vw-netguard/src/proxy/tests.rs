//! Transport-only tests for DIRECT and bypass order using loopback sockets as already-checked test addresses.
//! Policy integration is covered separately; this private module cannot provide a production bypass API.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
struct Resolver {
    values: Vec<String>,
    calls: AtomicUsize,
}
impl ProxyResolver for Resolver {
    fn resolve(&self, _: &str) -> Result<Vec<String>, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.values.clone())
    }
}

struct TrackingResolver(Mutex<Vec<String>>);
impl ProxyResolver for TrackingResolver {
    fn resolve(&self, url: &str) -> Result<Vec<String>, Error> {
        self.0.lock().unwrap().push(url.to_owned());
        Ok(vec!["direct://".into()])
    }
}
#[tokio::test]
async fn direct_fallback_is_explicit_and_stops_the_list() {
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let never = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = unavailable.local_addr().unwrap();
    drop(unavailable);
    let resolver = Arc::new(Resolver {
        values: vec![
            format!("http://{dead}"),
            "direct://".into(),
            format!("socks5://{}", never.local_addr().unwrap()),
        ],
        calls: AtomicUsize::new(0),
    });
    let upstream = Upstream::none().with_resolver(resolver);
    assert_eq!(upstream.last_route(), None);
    let url = url::Url::parse("https://allowed.test/").unwrap();
    let _opened = upstream
        .open(&url, true, &[direct.local_addr().unwrap()])
        .await
        .unwrap();
    assert_eq!(upstream.last_route(), Some(UsedRoute::Direct));
    tokio::time::timeout(Duration::from_secs(1), direct.accept())
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(25), never.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn successful_native_proxy_is_reported_without_its_address() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        assert!(request.starts_with(b"CONNECT 127.0.0.1:12345 HTTP/1.1\r\n"));
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
    });
    let upstream = Upstream::none().with_resolver(Arc::new(Resolver {
        values: vec![format!("http://{address}"), "direct://".into()],
        calls: AtomicUsize::new(0),
    }));
    let url = url::Url::parse("https://allowed.test/").unwrap();
    let _opened = upstream
        .open(&url, true, &["127.0.0.1:12345".parse().unwrap()])
        .await
        .unwrap();
    proxy.await.unwrap();
    assert_eq!(upstream.last_route(), Some(UsedRoute::Proxy));
}

#[tokio::test]
async fn failed_native_proxy_lookup_reports_no_successful_route() {
    let upstream = Upstream::none().with_resolver(Arc::new(Resolver {
        values: vec!["unsupported://proxy.test".into()],
        calls: AtomicUsize::new(0),
    }));
    let url = url::Url::parse("https://allowed.test/").unwrap();
    assert!(
        upstream
            .open(&url, true, &["127.0.0.1:12345".parse().unwrap()])
            .await
            .is_err()
    );
    assert_eq!(upstream.last_route(), None);
}

#[test]
fn system_route_is_only_selected_for_unconfigured_schemes() {
    let resolver = Arc::new(Resolver {
        values: vec!["direct://".into()],
        calls: AtomicUsize::new(0),
    });
    let settings = Upstream::from_values(Some("http://proxy.test:80"), None, None, None)
        .with_resolver(resolver.clone());
    assert!(!settings.uses_system_for("http"));
    assert!(settings.uses_system_for("https"));
    assert!(!settings.uses_system_for("wss"));
    assert!(!Upstream::none().uses_system_for("https"));
    assert!(
        !Upstream::from_values(None, None, Some("http://proxy.test:80"), None)
            .with_resolver(resolver)
            .uses_system_for("https")
    );
}

#[test]
fn configured_proxy_match_respects_scheme_bypass_and_invalid_values() {
    let external = url::Url::parse("https://allowed.test/").unwrap();
    let bypassed = url::Url::parse("https://inside.test/").unwrap();
    let http = url::Url::parse("http://allowed.test/").unwrap();
    let settings = Upstream::from_values(
        None,
        Some("http://proxy.test:8080"),
        None,
        Some("inside.test"),
    );
    assert_eq!(settings.configured_proxy_for(&external), Ok(true));
    assert_eq!(settings.configured_proxy_for(&bypassed), Ok(false));
    assert_eq!(settings.configured_proxy_for(&http), Ok(false));
    let all = Upstream::from_values(None, None, Some("http://proxy.test:8080"), None);
    assert_eq!(all.configured_proxy_for(&http), Ok(true));
    let malformed = Upstream::from_values(None, Some("invalid://proxy.test"), None, None);
    assert_eq!(
        malformed.configured_proxy_for(&external),
        Err(Error::Configuration)
    );
}

#[tokio::test]
async fn native_lookup_uses_its_target_url_without_changing_manual_scheme_matching() {
    let routing = url::Url::parse("https://allowed.test:80/").unwrap();
    let native = url::Url::parse("http://allowed.test:80/").unwrap();
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let resolver = Arc::new(TrackingResolver(Mutex::new(Vec::new())));
    let automatic = Upstream::none().with_resolver(resolver.clone());
    let _opened = automatic
        .open_with_system_url(&routing, &native, true, &[direct.local_addr().unwrap()])
        .await
        .unwrap();
    assert_eq!(resolver.0.lock().unwrap().as_slice(), &[native.as_str()]);

    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_address = dead.local_addr().unwrap();
    drop(dead);
    let resolver = Arc::new(TrackingResolver(Mutex::new(Vec::new())));
    let manual = Upstream::from_values(None, Some(&format!("http://{dead_address}")), None, None)
        .with_resolver(resolver.clone());
    assert!(
        manual
            .open_with_system_url(&routing, &native, true, &[direct.local_addr().unwrap()])
            .await
            .is_err()
    );
    assert!(resolver.0.lock().unwrap().is_empty());
}
#[tokio::test]
async fn saved_bypass_precedes_system_lookup_and_no_proxy_stays_direct() {
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let resolver = Arc::new(Resolver {
        values: vec!["invalid://bad".into()],
        calls: AtomicUsize::new(0),
    });
    let upstream = Upstream::from_values(None, None, None, Some("allowed.test"))
        .with_resolver(resolver.clone());
    let url = url::Url::parse("https://allowed.test/").unwrap();
    let _opened = upstream
        .open(&url, true, &[direct.local_addr().unwrap()])
        .await
        .unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    let _plain = Upstream::none()
        .open(&url, true, &[direct.local_addr().unwrap()])
        .await
        .unwrap();
}
#[tokio::test]
async fn malformed_settings_cannot_fall_back_to_system_or_direct() {
    let resolver = Arc::new(Resolver {
        values: vec!["direct://".into()],
        calls: AtomicUsize::new(0),
    });
    let upstream = Upstream::from_values(None, None, Some("invalid://user:secret@p"), None)
        .with_resolver(resolver.clone());
    assert!(matches!(
        upstream
            .routes(&url::Url::parse("https://allowed.test/").unwrap())
            .await,
        Err(Error::Configuration)
    ));
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_validated_proxy_is_never_silently_lost_by_a_second_parser() {
    let upstream = Upstream::from_values(None, None, Some("socks://proxy.test:1080"), None);
    let routes = upstream
        .routes(&url::Url::parse("https://allowed.test/").unwrap())
        .await
        .unwrap();
    assert!(
        routes[0].is_some(),
        "an accepted proxy alias must not become DIRECT"
    );
}

struct SlowResolver {
    active: AtomicUsize,
    peak: AtomicUsize,
}
impl ProxyResolver for SlowResolver {
    fn resolve(&self, _: &str) -> Result<Vec<String>, Error> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(vec!["direct://".into()])
    }
}
#[tokio::test]
async fn parallel_page_requests_wait_for_a_bounded_native_worker() {
    let resolver = Arc::new(SlowResolver {
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
    });
    let upstream = Arc::new(Upstream::none().with_resolver(resolver.clone()));
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let upstream = upstream.clone();
        requests.spawn(async move {
            upstream
                .routes(&url::Url::parse("https://allowed.test/").unwrap())
                .await
                .is_ok()
        });
    }
    while let Some(result) = requests.join_next().await {
        assert!(
            result.unwrap(),
            "parallel PAC requests must wait for a worker"
        );
    }
    assert!(resolver.peak.load(Ordering::SeqCst) <= 4);
}

/// PREDICT: every agent instruction below asked "the person"; the agent-facing actor word is "the user", so each
/// exact text differs until the outcomes say "Ask the user".
#[test]
fn every_proxy_outcome_tells_the_agent_to_ask_the_user() {
    assert_eq!(Error::ProxyUnreachable.state(), Error::Unreachable.state());
    let expected = [
        (
            Error::Configuration,
            "The configured proxy or PAC result is invalid or unsupported. Ask the user to check Settings > Network or the system proxy. Do not bypass the guard.",
        ),
        (
            Error::Discovery,
            "The OS proxy/PAC resolver failed or is unavailable. Ask the user to check the system proxy; on Linux install libproxy with the desktop/PAC backend or set a manual proxy in Settings > Network. Do not retry directly.",
        ),
        (
            Error::Unreachable,
            "The configured proxy could not be reached. Ask the user to check their network and proxy address, then retry. Do not bypass the proxy.",
        ),
        (
            Error::ProxyUnreachable,
            "The selected upstream proxy could not be reached. Ask the user to check their network and proxy address, then retry. Do not bypass the proxy.",
        ),
        (
            Error::Timeout,
            "The proxy connection or system PAC lookup timed out. Ask the user to check their proxy, then retry. Do not bypass the proxy.",
        ),
        (
            Error::Certificate,
            "TLS verification of the proxy failed. Ask the user to check the proxy name and company CA in Settings > Network. Never disable certificate verification.",
        ),
        (
            Error::Authentication,
            "The proxy rejected sign-in or requested an unsupported authentication method. Ask the user to check the proxy credentials in Settings or the system configuration. Never put credentials in chat or logs.",
        ),
        (
            Error::Refused,
            "The proxy refused the guard's checked IP destination. Ask the user to check the proxy's access rules. Never retry by hostname or bypass the guard.",
        ),
        (
            Error::Protocol,
            "The proxy returned an invalid handshake. Ask the user to check the proxy type and address, then retry. Do not bypass the guard.",
        ),
    ];
    for (error, text) in expected {
        assert_eq!(error.outcome(), text, "{error:?}");
    }
}
