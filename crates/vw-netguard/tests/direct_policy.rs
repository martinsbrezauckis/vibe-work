//! No-proxy compatibility and per-chat grants, exercised in both default and optional transport builds.
//! Depends only on the public guard API and a local HTTP server; never connects outside this computer.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "support/client.rs"]
mod proxy_client;

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use vw_netguard::{
    NetGuard, NoLoopback, PageContext, Scope, ScopeFuture, SystemResolver, Upstream,
};
struct Context(u16);
impl PageContext for Context {
    fn loopback<'a>(&'a self, key: &'a str, port: u16) -> ScopeFuture<'a> {
        if key == "owner" && port == self.0 {
            Box::pin(async { Ok(()) })
        } else {
            NoLoopback.loopback(port)
        }
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
        std::time::Duration::from_secs(2),
        client.read_to_string(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    response
}
#[tokio::test]
async fn no_proxy_keeps_direct_owner_grants_and_refuses_other_chats() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let guard = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
    guard.set_upstream(Upstream::none());
    guard.set_context(Arc::new(Context(port)));
    guard.serve();
    proxy_client::addr(&guard.proxy_url());
    let owner = guard.proxy_url_for_store("owner-store", "owner").unwrap();
    let other = guard.proxy_url_for_store("other-store", "other").unwrap();
    let owner: SocketAddr = proxy_client::addr(&owner);
    let other: SocketAddr = proxy_client::addr(&other);
    assert!(request(other, port).await.starts_with("HTTP/1.1 403"));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), listener.accept())
            .await
            .is_err()
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 1024];
        let n = socket.read(&mut bytes).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes[..n]).starts_with("GET / HTTP/1.1"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    let answer = request(owner, port).await;
    assert!(answer.starts_with("HTTP/1.1 200") && answer.ends_with("ok"));
    server.await.unwrap();
}
