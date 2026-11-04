//! Transport-only tests for DIRECT and bypass order using loopback sockets as already-checked test addresses.
//! Policy integration is covered separately; this private module cannot provide a production bypass API.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    let url = url::Url::parse("https://allowed.test/").unwrap();
    let _opened = upstream
        .open(&url, true, &[direct.local_addr().unwrap()])
        .await
        .unwrap();
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
