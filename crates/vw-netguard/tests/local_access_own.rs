//! This computer's own interface address is this computer for Local Network Access: a real server listening on every
//! address is reached through the computer's LAN address, and the guard treats that request like one to `127.0.0.1`
//! (the page's site is asked about "this computer", Vibe Work's own ports are refused whatever the answer).
//!
//! PREDICT (before the change): the own LAN address was classed by its range only (a private-network address), so
//! the question named the local network instead of this computer, and after Allow a request to one of Vibe Work's
//! own ports through that address was forwarded (the internal-port refusal looked at loopback only): the server saw
//! the request. `own_address_internal_port_is_refused_after_allow` fails at "reached Vibe Work"; the class check of
//! `own_address_is_asked_as_this_computer` fails with `Private`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "support/client.rs"]
mod proxy_client;

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use vw_netguard::ip::IpClass;
use vw_netguard::{LocalRequest, NetGuard, PageContext, ScopeFuture, SystemResolver, Upstream};

/// The own-address list is process-wide: one test at a time sets and uses it.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A person whose answer is fixed; records the class of each question.
struct Person {
    allow: bool,
    asked: Mutex<Vec<IpClass>>,
}

impl PageContext for Person {
    fn browser_addresses(&self, _: &str) -> bool {
        true
    }
    fn loopback<'a>(&'a self, _: &'a str, _: u16) -> ScopeFuture<'a> {
        Box::pin(async { Ok(()) })
    }
    fn local_access<'a>(
        &'a self,
        _: &'a [String],
        request: &'a LocalRequest<'a>,
    ) -> ScopeFuture<'a> {
        self.asked.lock().unwrap().push(request.class);
        let allow = self.allow;
        Box::pin(async move {
            if allow {
                Ok(())
            } else {
                Err("the person blocked this site".to_owned())
            }
        })
    }
}

/// The address this computer uses on its network (the source address of the default route; no packet is sent).
fn lan_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn setup(allow: bool, internal: Option<u16>) -> (NetGuard, Arc<Person>, SocketAddr) {
    let person = Arc::new(Person {
        allow,
        asked: Mutex::new(Vec::new()),
    });
    let guard = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
    guard.set_upstream(Upstream::none());
    guard.set_context(person.clone());
    if let Some(port) = internal {
        guard.add_internal_port(port);
    }
    guard.serve();
    let address = proxy_client::addr(&guard.proxy_url_for_store("web", "page").unwrap());
    (guard, person, address)
}

async fn get(proxy: SocketAddr, ip: IpAddr, port: u16) -> String {
    let target = SocketAddr::new(ip, port);
    let mut client = TcpStream::connect(proxy).await.unwrap();
    client
        .write_all(&proxy_client::authorize(
            proxy,
            format!(
                "GET http://{target}/data HTTP/1.1\r\nHost: {target}\r\nOrigin: https://public.example\r\n\r\n"
            )
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

/// Answers one request with 200 "ok" (the server on every address).
fn serve_once(listener: TcpListener) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 1024];
        let _ = socket.read(&mut bytes).await;
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await;
    })
}

#[tokio::test]
async fn own_address_internal_port_is_refused_after_allow() {
    let _one = ONE_AT_A_TIME.lock().await;
    let Some(ip) = lan_address() else {
        eprintln!("SKIP: this computer has no network address");
        return;
    };
    vw_netguard::own::set_own_addresses(vec![ip]);
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_guard, person, proxy) = setup(true, Some(port));
    let server = serve_once(listener);
    let answer = get(proxy, ip, port).await;
    assert!(
        answer.starts_with("HTTP/1.1 403"),
        "reached Vibe Work through {ip}: {answer}"
    );
    assert!(answer.contains("belongs to Vibe Work itself"), "{answer}");
    assert!(
        person.asked.lock().unwrap().is_empty(),
        "never asked about an internal port"
    );
    assert!(!server.is_finished(), "the server saw a request");
    server.abort();
}

#[tokio::test]
async fn own_address_is_asked_as_this_computer() {
    let _one = ONE_AT_A_TIME.lock().await;
    let Some(ip) = lan_address() else {
        eprintln!("SKIP: this computer has no network address");
        return;
    };
    vw_netguard::own::set_own_addresses(vec![ip]);
    // Allowed: the real server answers through the LAN address.
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_guard, person, proxy) = setup(true, None);
    let server = serve_once(listener);
    let answer = get(proxy, ip, port).await;
    assert!(
        answer.starts_with("HTTP/1.1 200") && answer.ends_with("ok"),
        "{answer}"
    );
    server.await.unwrap();
    assert_eq!(
        *person.asked.lock().unwrap(),
        vec![IpClass::Loopback],
        "asked as this computer"
    );
    // Blocked: refused, nothing connected.
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_guard, _person, proxy) = setup(false, None);
    let answer = get(proxy, ip, port).await;
    assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), listener.accept())
            .await
            .is_err(),
        "connected while blocked"
    );
}

#[tokio::test]
async fn own_addresses_are_kept_canonical_and_change_the_rules_revision() {
    let _one = ONE_AT_A_TIME.lock().await;
    let before = vw_netguard::rules::revision();
    let changed = vw_netguard::own::set_own_addresses(vec![
        "192.0.2.77".parse().unwrap(),
        "::ffff:192.0.2.77".parse().unwrap(),
        "127.0.0.1".parse().unwrap(),
        "::".parse().unwrap(),
        "2001:db8::77".parse().unwrap(),
    ]);
    assert!(changed);
    assert_ne!(vw_netguard::rules::revision(), before);
    assert!(vw_netguard::own::is_own("192.0.2.77".parse().unwrap()));
    assert!(vw_netguard::own::is_own(
        "::ffff:192.0.2.77".parse().unwrap()
    ));
    assert!(vw_netguard::own::is_own("2001:db8::77".parse().unwrap()));
    assert!(
        !vw_netguard::own::is_own("127.0.0.1".parse().unwrap()),
        "loopback by class already"
    );
    assert!(!vw_netguard::own::is_own("192.0.2.78".parse().unwrap()));
}
