//! Explicit proxy credentials for existing socket policy fixtures; ingress rejection tests send raw requests.
//! Records capabilities returned by the guard and adds one header to the first request only, never tunnel data.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use base64::Engine as _;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};

fn credentials() -> &'static Mutex<HashMap<SocketAddr, String>> {
    static VALUES: OnceLock<Mutex<HashMap<SocketAddr, String>>> = OnceLock::new();
    VALUES.get_or_init(Mutex::default)
}

pub fn addr(proxy: &str) -> SocketAddr {
    let url = url::Url::parse(proxy).unwrap();
    let address = SocketAddr::new("127.0.0.1".parse().unwrap(), url.port().unwrap());
    credentials().lock().unwrap().insert(address, header(proxy));
    address
}

pub fn header(proxy: &str) -> String {
    let url = url::Url::parse(proxy).unwrap();
    format!(
        "Proxy-Authorization: Basic {}\r\n",
        base64::engine::general_purpose::STANDARD.encode(format!(
            "{}:{}",
            url.username(),
            url.password().unwrap()
        ))
    )
}

pub fn authorize(address: SocketAddr, request: &[u8]) -> Vec<u8> {
    let header = credentials()
        .lock()
        .unwrap()
        .get(&address)
        .cloned()
        .expect("register the proxy URL first");
    let end = request.windows(2).position(|w| w == b"\r\n").unwrap() + 2;
    [&request[..end], header.as_bytes(), &request[end..]].concat()
}
