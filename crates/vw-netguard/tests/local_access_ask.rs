//! Local Network Access through a real guard listener: the core's answer decides, nothing is connected before it,
//! the request's initiator reaches the answer, and Vibe Work's own ports stay refused whatever it says.
//!
//! PREDICT (before the change): there was no `local_access` question; a browser store forwarded a loopback request at
//! once (the local server saw it before any answer could exist) and an internal port was forwarded like any other.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "support/client.rs"]
mod proxy_client;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use vw_netguard::decide::decide_for;
use vw_netguard::ip::IpClass;
use vw_netguard::{
    LocalRequest, NetGuard, PageContext, Scope, ScopeFuture, SystemResolver, Upstream,
};

/// What the context was asked: (members, class, port, initiator).
type Asked = (Vec<String>, IpClass, u16, Option<String>);

/// A context whose answer the test gives later (`None` while open).
struct Person {
    answer: watch::Receiver<Option<bool>>,
    asked: Mutex<Vec<Asked>>,
    /// Whether each asked request was marked as a nested document.
    nested: Mutex<Vec<bool>>,
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
        keys: &'a [String],
        request: &'a LocalRequest<'a>,
    ) -> ScopeFuture<'a> {
        self.asked.lock().unwrap().push((
            keys.to_vec(),
            request.class,
            request.port,
            request.initiator.map(str::to_owned),
        ));
        self.nested.lock().unwrap().push(request.nested);
        let mut answer = self.answer.clone();
        Box::pin(async move {
            let given = answer
                .wait_for(Option::is_some)
                .await
                .map_err(|_| "no answer".to_owned())?;
            if given.unwrap_or(false) {
                Ok(())
            } else {
                Err("the person blocked this site".to_owned())
            }
        })
    }
}

async fn send(address: SocketAddr, port: u16, extra: &str) -> TcpStream {
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(&proxy_client::authorize(
            address,
            format!(
                "GET http://127.0.0.1:{port}/data HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra}\r\n"
            )
            .as_bytes(),
        ))
        .await
        .unwrap();
    client
}

async fn read_all(mut client: TcpStream) -> String {
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

fn setup(internal: Option<u16>) -> (NetGuard, Arc<Person>, watch::Sender<Option<bool>>) {
    let (tell, answer) = watch::channel(None);
    let person = Arc::new(Person {
        answer,
        asked: Mutex::new(Vec::new()),
        nested: Mutex::new(Vec::new()),
    });
    let guard = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
    guard.set_upstream(Upstream::none());
    guard.set_context(person.clone());
    if let Some(port) = internal {
        guard.add_internal_port(port);
    }
    guard.serve();
    (guard, person, tell)
}

#[tokio::test]
async fn the_request_waits_for_the_answer_and_allow_lets_it_through() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (guard, person, tell) = setup(None);
    let address = proxy_client::addr(&guard.proxy_url_for_store("web", "page").unwrap());
    let client = send(address, port, "Origin: https://public.example\r\n").await;
    // Open question: nothing reaches the local server.
    assert!(
        tokio::time::timeout(Duration::from_millis(400), listener.accept())
            .await
            .is_err(),
        "sent before the answer"
    );
    assert_eq!(
        *person.asked.lock().unwrap(),
        vec![(
            vec!["page".to_owned()],
            IpClass::Loopback,
            port,
            Some("https://public.example".to_owned())
        )]
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 1024];
        let n = socket.read(&mut bytes).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes[..n]).starts_with("GET /data HTTP/1.1"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    tell.send_replace(Some(true));
    let answer = read_all(client).await;
    assert!(
        answer.starts_with("HTTP/1.1 200") && answer.ends_with("ok"),
        "{answer}"
    );
    server.await.unwrap();
}

/// The engine's `Sec-Fetch-Dest` reaches the answer: a frame's or another nested document is marked, the page's own
/// document and a request without the header are not.
/// PREDICT (before the field): the guard drops the header, so a frame's navigation looks like the page's own.
#[tokio::test]
async fn a_nested_document_is_marked_for_the_answer() {
    let (guard, person, tell) = setup(None);
    tell.send_replace(Some(false));
    let address = proxy_client::addr(&guard.proxy_url_for_store("web", "page").unwrap());
    for extra in [
        "Sec-Fetch-Dest: iframe\r\n",
        "sec-fetch-dest: embed\r\n",
        "Sec-Fetch-Dest: document\r\n",
        "",
    ] {
        let _ = read_all(send(address, 9, extra).await).await;
    }
    assert_eq!(
        *person.nested.lock().unwrap(),
        vec![true, true, false, false]
    );
}

/// A CONNECT tunnel shows no `Sec-Fetch-Dest`. On Linux the engine's navigation callback also reports a frame's
/// navigation as the page's, so nothing names a tunnel as the page's own top-level navigation: it reaches the answer
/// marked like a frame's document and is asked like the page's other requests. Engines whose callback reports only
/// the page's navigation (Windows, macOS) keep the exemption for a tunnel to the address the page navigates to.
/// PREDICT (before the change): every tunnel is reported `nested: false`, so a public page's HTTPS iframe
/// to a local TLS server took the page's top-level navigation exemption on Linux; this test's Linux row fails.
#[tokio::test]
async fn a_tunnel_is_never_the_pages_own_navigation_on_linux() {
    let (guard, person, tell) = setup(None);
    tell.send_replace(Some(false));
    let address = proxy_client::addr(&guard.proxy_url_for_store("web", "page").unwrap());
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(&proxy_client::authorize(
            address,
            b"CONNECT 127.0.0.1:9 HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n",
        ))
        .await
        .unwrap();
    let answer = read_all(client).await;
    assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
    assert_eq!(
        *person.nested.lock().unwrap(),
        vec![cfg!(target_os = "linux")]
    );
}

#[tokio::test]
async fn block_refuses_without_connecting() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (guard, _person, tell) = setup(None);
    let address = proxy_client::addr(&guard.proxy_url_for_store("web", "page").unwrap());
    let client = send(address, port, "Referer: http://public.example/page?q=1\r\n").await;
    tell.send_replace(Some(false));
    let answer = read_all(client).await;
    assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
    assert!(answer.contains("blocked this site"), "{answer}");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn internal_ports_are_refused_even_when_the_answer_allows() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (guard, person, tell) = setup(Some(port));
    tell.send_replace(Some(true));
    let address = proxy_client::addr(&guard.proxy_url_for_store("web", "page").unwrap());
    let answer = read_all(send(address, port, "").await).await;
    assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
    assert!(answer.contains("belongs to Vibe Work itself"), "{answer}");
    assert!(
        person.asked.lock().unwrap().is_empty(),
        "never asked about an internal port"
    );
}

/// The class and initiator each target reports, at the `decide` level (no sockets).
#[tokio::test]
async fn every_non_public_class_is_asked_and_public_is_not() {
    #[derive(Default)]
    struct Record(Mutex<Vec<(IpClass, Option<String>)>>);
    impl Scope for Record {
        fn browser_addresses(&self) -> bool {
            true
        }
        fn loopback(&self, _port: u16) -> ScopeFuture<'_> {
            Box::pin(async { Ok(()) })
        }
        fn local_access<'a>(&'a self, request: &'a LocalRequest<'a>) -> ScopeFuture<'a> {
            self.0
                .lock()
                .unwrap()
                .push((request.class, request.initiator.map(str::to_owned)));
            Box::pin(async { Ok(()) })
        }
    }
    let resolver = SystemResolver;
    let cases = [
        ("127.0.0.1", IpClass::Loopback),
        ("localhost", IpClass::Loopback),
        ("0.0.0.0", IpClass::Loopback),
        ("[::ffff:127.0.0.1]", IpClass::Loopback),
        ("10.20.30.40", IpClass::Private),
        ("10.1.2.3", IpClass::Private),
        ("169.254.169.254", IpClass::LinkLocal),
        ("[fe80::1]", IpClass::LinkLocal),
        ("100.64.1.1", IpClass::SharedCgnat),
    ];
    for (host, class) in cases {
        let scope = Record::default();
        let addrs = decide_for(
            &resolver,
            &scope,
            host,
            80,
            Some("https://a.example"),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            addrs.len(),
            if host == "localhost" { 2 } else { 1 },
            "{host}"
        );
        assert_eq!(
            *scope.0.lock().unwrap(),
            vec![(class, Some("https://a.example".to_owned()))],
            "{host}"
        );
    }
    let scope = Record::default();
    decide_for(&resolver, &scope, "93.184.216.34", 443, None, false)
        .await
        .unwrap();
    assert!(scope.0.lock().unwrap().is_empty(), "public is never asked");
}
