//! Serving boundary for handbook controls: a script-free host and opaque prototype, including scripts-off fallback.
//! Depends on the loopback page server; scoped Cargo checks call it. Restoring byte-identical prototype serving falsifies it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use vw_pages::PagesServer;

const PAGE: &str = "<!doctype html><html><head><title>Board</title><meta name=\"handbook.kind\" content=\"prototype\"></head>\
    <body><div class=\"hb-input\" id=\"release\" data-input=\"choice\"><p class=\"hb-question\">Release</p>\
    <ul><li data-option=\"now\">Now</li></ul></div><script>window.tryAnswer=true</script></body></html>";

async fn get(server: &PagesServer, path: &str) -> String {
    let port = server.port();
    let base = url::Url::parse(&server.base_url()).unwrap();
    let path = format!("{}{path}", base.path().trim_end_matches('/'));
    tokio::task::spawn_blocking(move || {
        let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            socket,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn prototype_scripts_never_share_the_inline_controls_document() {
    let root = std::env::temp_dir().join(format!("vw-inline-security-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("board.html"), PAGE).unwrap();
    let server = PagesServer::start(root.clone(), true, "Board".into())
        .await
        .unwrap();
    let host = get(&server, "/board.html").await;
    let (headers, body) = host.split_once("\r\n\r\n").unwrap();
    assert!(headers.contains("script-src 'none'"));
    assert!(headers.contains(&format!("frame-src {}board.html", server.base_url())));
    assert!(body.contains("sandbox=\"allow-scripts\""));
    assert!(!body.contains("window.tryAnswer"));
    let frame = get(&server, "/board.html?hb-body").await;
    assert!(frame.contains("sandbox allow-scripts"));
    assert!(frame.contains("connect-src 'none'"));
    assert!(frame.contains("frame-src 'none'"));
    assert!(frame.contains("window.tryAnswer"));
    assert!(frame.contains("hb-layout"));
    server.set_scripts(false);
    let off = get(&server, "/board.html").await;
    let (headers, body) = off.split_once("\r\n\r\n").unwrap();
    assert!(headers.contains("script-src 'none'"));
    assert!(headers.contains("frame-src 'none'"));
    assert_eq!(
        body, PAGE,
        "scripts-off pages retain static input positions"
    );
    std::fs::remove_dir_all(root).unwrap();
}
