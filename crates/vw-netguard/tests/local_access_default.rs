//! A live browser store whose context gives no Local Network Access answer reaches only public addresses: this
//! computer, the local network and link-local addresses are refused before any connection, and Vibe Work's own ports
//! are refused for browser pages too.
//!
//! PREDICT (before the change): `decide` admitted every address for a browser scope and never consulted the context,
//! so a page on the shared web store reached `127.0.0.1:<port>` (the local server accepted the connection and the
//! proxy answered 200), `10.20.30.40` and `169.254.169.254` were admitted, and an internal port was forwarded. These
//! tests then fail on the 200 answer, the accepted connection and the admitted addresses.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "support/client.rs"]
mod proxy_client;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use vw_netguard::decide::decide;
use vw_netguard::{NetGuard, PageContext, Scope, ScopeFuture, SystemResolver, Upstream};

/// A browser page context that answers nothing about local access (the default).
struct Browser;
impl PageContext for Browser {
    fn browser_addresses(&self, _: &str) -> bool {
        true
    }
    fn loopback<'a>(&'a self, _: &'a str, _: u16) -> ScopeFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

async fn request(address: SocketAddr, port: u16) -> String {
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(&proxy_client::authorize(
            address,
            format!("GET http://127.0.0.1:{port}/ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")
                .as_bytes(),
        ))
        .await
        .unwrap();
    let mut response = String::new();
    tokio::time::timeout(
        Duration::from_secs(10),
        client.read_to_string(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    response
}

#[tokio::test]
async fn a_browser_store_without_an_answer_does_not_reach_this_computer() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let guard = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
    guard.set_upstream(Upstream::none());
    guard.set_context(Arc::new(Browser));
    guard.serve();
    let address = proxy_client::addr(&guard.proxy_url_for_store("web", "page").unwrap());
    let answer = request(address, port).await;
    assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), listener.accept())
            .await
            .is_err(),
        "nothing may connect to the local server"
    );
}

#[tokio::test]
async fn vibe_work_ports_are_refused_for_browser_pages() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let guard = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
    guard.set_upstream(Upstream::none());
    guard.set_context(Arc::new(Browser));
    guard.add_internal_port(port);
    guard.serve();
    let address = proxy_client::addr(&guard.proxy_url_for_store("web", "page").unwrap());
    let answer = request(address, port).await;
    assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
    assert!(answer.contains("belongs to Vibe Work itself"), "{answer}");
}

#[tokio::test]
async fn local_network_and_link_local_literals_are_refused_without_an_answer() {
    struct Page;
    impl Scope for Page {
        fn browser_addresses(&self) -> bool {
            true
        }
        fn loopback(&self, _port: u16) -> ScopeFuture<'_> {
            Box::pin(async { Ok(()) })
        }
    }
    let resolver = SystemResolver;
    for host in [
        "127.0.0.1",
        "localhost",
        "0.0.0.0",
        "10.20.30.40",
        "10.1.2.3",
        "172.16.1.2",
        "169.254.169.254",
        "[fe80::1]",
        "[::1]",
    ] {
        assert!(
            decide(&resolver, &Page, host, 8080).await.is_err(),
            "{host} must not be admitted without an answer"
        );
    }
    assert!(decide(&resolver, &Page, "93.184.216.34", 443).await.is_ok());
}
