//! Every guard's listeners are Vibe Work's own ports for every page: a second guard in the process (the vault's
//! proxy forwarders, a connection test) is refused through the browser guard's proxy (Linux, Windows and macOS
//! host names) and held by the browser content rules (macOS address literals), also after the person allowed the
//! site, without the second guard registering anything. A guard's ports leave the set when it shuts down.
//!
//! PREDICT (before the process-wide listener set): only the publishing guard's own listeners and its registered
//! ports reach `rules::current_inputs().internal_ports` and its `is_internal`, so the second guard's shared and store
//! ports are missing from the content rules and an allowed page reaches them through the browser guard (the proxy
//! answers with the forwarded request, not "belongs to Vibe Work itself").
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "support/client.rs"]
mod proxy_client;

use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use vw_netguard::rules::{self, RulePage};
use vw_netguard::{LocalRequest, NetGuard, PageContext, ScopeFuture, SystemResolver, Upstream};

/// A person who allows every site and every loopback port.
struct AllowAll;

impl PageContext for AllowAll {
    fn browser_addresses(&self, _: &str) -> bool {
        true
    }
    fn loopback<'a>(&'a self, _: &'a str, _: u16) -> ScopeFuture<'a> {
        Box::pin(async { Ok(()) })
    }
    fn local_access<'a>(&'a self, _: &'a [String], _: &'a LocalRequest<'a>) -> ScopeFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

/// A second guard set up like the vault's proxy forwarder (no canaries, no upstream, publishes nothing).
fn forwarder() -> NetGuard {
    let guard = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
    guard.disable_canaries();
    guard.set_upstream(Upstream::none());
    guard.serve();
    guard
}

fn browser() -> NetGuard {
    let guard = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
    guard.set_upstream(Upstream::none());
    guard.set_context(Arc::new(AllowAll));
    guard.publish_internal_ports();
    guard.serve();
    guard
}

fn port_of(proxy: &str) -> u16 {
    url::Url::parse(proxy).unwrap().port().unwrap()
}

#[tokio::test]
async fn every_guards_listeners_are_held_by_the_content_rules_until_it_shuts_down() {
    let browser = browser();
    let before = rules::revision();
    let other = forwarder();
    let other_store = port_of(&other.store_proxy_url("probe").unwrap());
    let ports = [browser.addr().port(), other.addr().port(), other_store];
    assert_ne!(
        rules::revision(),
        before,
        "a new guard moves the rules' revision"
    );
    let held = rules::current_inputs().internal_ports;
    let text = rules::content_rules_json(&RulePage::Web);
    for port in ports {
        assert!(held.contains(&port), "{port} missing from {held:?}");
        assert!(
            text.contains(&format!(":{port}[/?#]")),
            "no content rule holds {port}"
        );
        assert!(
            browser.internal_ports().contains(&port),
            "{port} not internal to the browser guard"
        );
        assert!(
            other.internal_ports().contains(&port),
            "{port} not internal to the other guard"
        );
    }
    let (shared, store) = (other.addr().port(), other_store);
    other.shutdown();
    let held = rules::current_inputs().internal_ports;
    assert!(
        !held.contains(&shared) && !held.contains(&store),
        "a shut-down guard's ports stay held: {held:?}"
    );
    assert!(held.contains(&browser.addr().port()));
}

#[tokio::test]
async fn an_allowed_page_never_reaches_another_guards_listener_through_the_browser_guard() {
    let browser = browser();
    let other = forwarder();
    let address = proxy_client::addr(&browser.proxy_url_for_store("web", "page").unwrap());
    for port in [
        other.addr().port(),
        port_of(&other.store_proxy_url("probe").unwrap()),
    ] {
        let mut client = TcpStream::connect(address).await.unwrap();
        let request = format!(
            "GET http://127.0.0.1:{port}/ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: https://public.example\r\n\r\n"
        );
        client
            .write_all(&proxy_client::authorize(address, request.as_bytes()))
            .await
            .unwrap();
        let mut answer = String::new();
        tokio::time::timeout(Duration::from_secs(10), client.read_to_string(&mut answer))
            .await
            .unwrap()
            .unwrap();
        assert!(answer.starts_with("HTTP/1.1 403"), "{port}: {answer}");
        assert!(
            answer.contains("belongs to Vibe Work itself"),
            "{port}: {answer}"
        );
    }
}

/// One request through `address` (with its credential); the whole answer.
async fn exchange(address: std::net::SocketAddr, request: &str) -> String {
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(&proxy_client::authorize(address, request.as_bytes()))
        .await
        .unwrap();
    let mut answer = String::new();
    tokio::time::timeout(Duration::from_secs(10), client.read_to_string(&mut answer))
        .await
        .unwrap()
        .unwrap();
    answer
}

/// The guard's diagnostic canary at its own listener port answers only the probe that announced it, while that probe
/// waits; any other request for the port (a page's subresource with the browser's proxy credential, a tunnel) is
/// refused like every page's request for Vibe Work's own ports, and marks nothing.
/// PREDICT (before the change): `serve_one` answered every `/__vw_guard/<token>` and every tunnel to the
/// listener's own port with `200 ... loopback active` and marked it seen, before the own-port refusal.
#[tokio::test]
async fn the_own_port_canary_answers_only_the_announced_probe() {
    let browser = browser();
    let proxy = browser.proxy_url_for_store("web", "page").unwrap();
    let (address, port) = (proxy_client::addr(&proxy), port_of(&proxy));
    let canary = |token: &str| {
        let url = NetGuard::loopback_canary_url(&proxy, token);
        format!("GET {url} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")
    };
    let tunnel = format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    for request in [canary("chosen-by-the-page"), tunnel.clone()] {
        let answer = exchange(address, &request).await;
        assert!(answer.starts_with("HTTP/1.1 403"), "{request}: {answer}");
        assert!(answer.contains("belongs to Vibe Work itself"), "{answer}");
    }
    assert!(
        !browser
            .wait_seen_loopback("chosen-by-the-page", port, Duration::from_millis(200))
            .await,
        "an unannounced canary marks nothing"
    );

    let token = NetGuard::new_token();
    browser.announce_loopback(&token, port);
    let answer = exchange(address, &canary(&token)).await;
    assert!(answer.contains("loopback active"), "{answer}");
    assert!(
        browser
            .wait_seen_loopback(&token, port, Duration::from_secs(6))
            .await
    );
    // The probe ended: its token and the tunnel are refused again.
    for request in [canary(&token), tunnel] {
        let answer = exchange(address, &request).await;
        assert!(answer.starts_with("HTTP/1.1 403"), "{request}: {answer}");
    }
}
