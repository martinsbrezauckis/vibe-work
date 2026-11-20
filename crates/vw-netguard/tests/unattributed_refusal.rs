//! A refusal on a store with several pages reaches every page that could have sent it.
//!
//! PREDICT (fails before the change): the guard recorded a refusal under a page only when the store had one member;
//! with two pages open the refusal of a request that named no initiator was recorded under none, so no page's agent
//! receipt carried it and the native 403 read as the server's answer.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "support/client.rs"]
mod proxy_client;

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use vw_netguard::{LocalRequest, NetGuard, PageContext, ScopeFuture, SystemResolver, Upstream};

/// Refuses every local request and records what the guard reported.
#[derive(Default)]
struct Context {
    refused: Mutex<Vec<Option<String>>>,
    unattributed: Mutex<Vec<Vec<String>>>,
}

impl PageContext for Context {
    fn browser_addresses(&self, _: &str) -> bool {
        true
    }
    fn loopback<'a>(&'a self, _: &'a str, _: u16) -> ScopeFuture<'a> {
        Box::pin(async { Ok(()) })
    }
    fn local_access<'a>(&'a self, _: &'a [String], _: &'a LocalRequest<'a>) -> ScopeFuture<'a> {
        Box::pin(async { Err("several web pages are open in the browser".to_owned()) })
    }
    fn refused(&self, key: Option<&str>, _: &str, _: u16, _: &str) {
        self.refused.lock().unwrap().push(key.map(str::to_owned));
    }
    fn refused_unattributed(&self, keys: &[String], _: &str, _: u16, _: &str) {
        let mut keys = keys.to_vec();
        keys.sort();
        self.unattributed.lock().unwrap().push(keys);
    }
}

async fn refused_by(guard: &NetGuard, label: &str, port: u16) {
    let address = proxy_client::addr(&guard.proxy_url_for_store("web", label).unwrap());
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(&proxy_client::authorize(
            address,
            format!("GET http://127.0.0.1:{port}/r.png HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")
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
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
}

#[tokio::test]
async fn an_unattributed_refusal_names_every_page_of_the_store() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let context = Arc::new(Context::default());
    let guard = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
    guard.set_upstream(Upstream::none());
    guard.set_context(context.clone());
    guard.serve();
    // One page: the refusal is that page's, no store-wide report.
    refused_by(&guard, "one", port).await;
    assert_eq!(
        *context.refused.lock().unwrap(),
        vec![Some("one".to_owned())]
    );
    assert!(context.unattributed.lock().unwrap().is_empty());
    // A second page on the same store: the request can be either page's.
    refused_by(&guard, "two", port).await;
    assert_eq!(context.refused.lock().unwrap().last(), Some(&None));
    assert_eq!(
        *context.unattributed.lock().unwrap(),
        vec![vec!["one".to_owned(), "two".to_owned()]]
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), listener.accept())
            .await
            .is_err(),
        "a refused request reached the server"
    );
}
