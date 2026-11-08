//! Local proxy transport regressions. Depends on tokio and the public guard API; cargo test runs these.
//! No Internet: fake proxies echo bytes and record the numeric destination instead of dialing it.
#![cfg(feature = "proxy-transports")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/client.rs"]
mod proxy_client;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use vw_netguard::{NetGuard, Resolve, ResolveFuture, Upstream};

struct Dns;
impl Resolve for Dns {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        let ip = if host == "allowed.test" {
            "93.184.215.14"
        } else {
            "10.0.0.1"
        };
        Box::pin(async move { Ok(vec![SocketAddr::new(ip.parse().unwrap(), port)]) })
    }
}

async fn head(s: &mut TcpStream) -> String {
    let mut out = Vec::new();
    while !out.ends_with(b"\r\n\r\n") {
        out.push(s.read_u8().await.unwrap());
    }
    String::from_utf8(out).unwrap()
}

#[tokio::test]
async fn socks_remote_dns_receives_only_the_checked_ip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("socks5h://{}", listener.local_addr().unwrap());
    let guard = NetGuard::bind(Arc::new(Dns)).unwrap();
    guard.set_upstream(Upstream::from_values(None, Some(&proxy), None, None));
    guard.serve();
    proxy_client::addr(&guard.proxy_url());
    let address: SocketAddr = proxy_client::addr(&guard.proxy_url());
    let mut denied = TcpStream::connect(address).await.unwrap();
    denied
        .write_all(&proxy_client::authorize(
            address,
            b"CONNECT denied.test:443 HTTP/1.1\r\nHost: denied.test:443\r\n\r\n",
        ))
        .await
        .unwrap();
    assert!(head(&mut denied).await.starts_with("HTTP/1.1 403"));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(&proxy_client::authorize(
            address,
            b"CONNECT allowed.test:443 HTTP/1.1\r\nHost: allowed.test:443\r\n\r\n",
        ))
        .await
        .unwrap();
    let (mut upstream, _) = tokio::time::timeout(Duration::from_secs(1), listener.accept())
        .await
        .expect("SOCKS proxy must receive the allowed connection")
        .unwrap();
    let mut hello = [0; 3];
    upstream.read_exact(&mut hello).await.unwrap();
    assert_eq!(hello, [5, 1, 0]);
    upstream.write_all(&[5, 0]).await.unwrap();
    let mut request = [0; 10];
    upstream.read_exact(&mut request).await.unwrap();
    assert_eq!(request, [5, 1, 0, 1, 93, 184, 215, 14, 1, 187]);
    upstream
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    assert!(head(&mut client).await.starts_with("HTTP/1.1 200"));
}

#[tokio::test]
async fn https_proxy_starts_with_tls() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("https://{}", listener.local_addr().unwrap());
    let guard = NetGuard::bind(Arc::new(Dns)).unwrap();
    guard.set_upstream(Upstream::from_values(None, Some(&proxy), None, None));
    guard.serve();
    proxy_client::addr(&guard.proxy_url());
    let address: SocketAddr = proxy_client::addr(&guard.proxy_url());
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(&proxy_client::authorize(
            address,
            b"CONNECT allowed.test:443 HTTP/1.1\r\nHost: allowed.test:443\r\n\r\n",
        ))
        .await
        .unwrap();
    let (mut upstream, _) = tokio::time::timeout(Duration::from_secs(1), listener.accept())
        .await
        .expect("HTTPS proxy must receive the allowed connection")
        .unwrap();
    assert_eq!(upstream.read_u8().await.unwrap(), 0x16);
}
