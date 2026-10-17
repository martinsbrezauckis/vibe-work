//! The viewer's loopback server refuses documents and sibling assets without its path capability.
//! Depends on `PagesServer` and raw TCP; scoped Cargo checks exercise the serving boundary without a window.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::path::PathBuf;
use url::Url;
use vw_pages::PagesServer;

async fn get(server: &PagesServer, path: &str) -> Vec<u8> {
    let port = server.port();
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        write!(
            socket,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).unwrap();
        response
    })
    .await
    .unwrap()
}

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("vw-capability-{name}-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/start.html"),
        "<!doctype html><title>Private page</title><link rel=\"stylesheet\" href=\"../style.css\">\
         <img src=\"../image.png\"><a href=\"next.html\">Next</a>",
    )
    .unwrap();
    std::fs::write(root.join("pages/next.html"), "<title>Next page</title>").unwrap();
    std::fs::write(root.join("style.css"), "body { color: blue; }").unwrap();
    std::fs::write(root.join("image.png"), b"\x89PNG\r\n\x1a\nprivate image").unwrap();
    std::fs::write(root.join("doc.pdf"), b"%PDF-1.4\nprivate PDF\n%%EOF\n").unwrap();
    root
}

#[tokio::test]
async fn raw_client_cannot_read_a_handbook_page() {
    let root = fixture("page");
    let server = PagesServer::start(root.clone(), true, "Private".into())
        .await
        .unwrap();
    let response = get(&server, "/pages/start.html").await;
    assert!(
        response.starts_with(b"HTTP/1.1 404"),
        "{:?}",
        String::from_utf8_lossy(&response)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn raw_client_cannot_read_sibling_assets() {
    let root = fixture("assets");
    let server = PagesServer::start(root.clone(), true, "Private".into())
        .await
        .unwrap();
    for path in ["/", "/style.css", "/image.png", "/pages/start.html?hb-body"] {
        let response = get(&server, path).await;
        assert!(
            response.starts_with(b"HTTP/1.1 404"),
            "{path}: {:?}",
            String::from_utf8_lossy(&response)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn raw_client_cannot_read_a_file_viewer_pdf() {
    let root = fixture("pdf");
    let server = PagesServer::start_for_files(root.clone(), "Private".into())
        .await
        .unwrap();
    for path in ["/doc.pdf", "/pages/start.html", "/style.css", "/image.png"] {
        let response = get(&server, path).await;
        assert!(
            response.starts_with(b"HTTP/1.1 404"),
            "{path}: {:?}",
            String::from_utf8_lossy(&response)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn authorized_urls_keep_relative_pages_assets_and_pdf() {
    let root = fixture("authorized");
    let handbook = PagesServer::start(root.clone(), true, "Private".into())
        .await
        .unwrap();
    let files = PagesServer::start_for_files(root.clone(), "Private".into())
        .await
        .unwrap();
    for server in [&handbook, &files] {
        let base = Url::parse(&server.base_url()).unwrap();
        let page = base.join("pages/start.html").unwrap();
        for (relative, file, mime) in [
            ("start.html", "pages/start.html", "text/html; charset=utf-8"),
            ("../style.css", "style.css", "text/css; charset=utf-8"),
            ("../image.png", "image.png", "image/png"),
            ("next.html", "pages/next.html", "text/html; charset=utf-8"),
        ] {
            let url = page.join(relative).unwrap();
            let response = get(server, url.path()).await;
            assert!(response.starts_with(b"HTTP/1.1 200"), "{relative}");
            let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            let headers = String::from_utf8_lossy(&response[..split]);
            assert!(
                headers.contains(&format!("content-type: {mime}\r\n")),
                "{headers}"
            );
            assert_eq!(
                &response[split + 4..],
                std::fs::read(root.join(file)).unwrap()
            );
        }
    }
    let pdf = Url::parse(&files.base_url())
        .unwrap()
        .join("doc.pdf")
        .unwrap();
    let response = get(&files, pdf.path()).await;
    assert!(response.starts_with(b"HTTP/1.1 200"));
    assert!(String::from_utf8_lossy(&response).contains("content-type: application/pdf\r\n"));
    assert!(response.ends_with(&std::fs::read(root.join("doc.pdf")).unwrap()));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn wrong_capabilities_and_unprefixed_check_routes_are_refused() {
    let root = fixture("wrong");
    let server = PagesServer::start(root.clone(), true, "Private".into())
        .await
        .unwrap();
    let other = PagesServer::start(root.clone(), true, "Private".into())
        .await
        .unwrap();
    let base = Url::parse(&server.base_url()).unwrap();
    let capability = base.path_segments().unwrap().next().unwrap();
    assert_eq!(capability.len(), 64);
    assert!(capability.bytes().all(|b| b.is_ascii_hexdigit()));
    let other_base = Url::parse(&other.base_url()).unwrap();
    assert_ne!(base.path(), other_base.path());
    let mut bad = vec![
        "/pages/start.html".to_owned(),
        format!("/{capability}"),
        format!("/{capability}x/pages/start.html"),
        format!(
            "{}/pages/start.html",
            other_base.path().trim_end_matches('/')
        ),
        format!("/{}/pages/start.html", server.check_prefix()),
        format!("/{}/", server.check_prefix()),
    ];
    for position in [0, capability.len() / 2, capability.len() - 1] {
        let mut wrong = capability.as_bytes().to_vec();
        wrong[position] = if wrong[position] == b'0' { b'1' } else { b'0' };
        bad.push(format!(
            "/{}/pages/start.html",
            String::from_utf8(wrong).unwrap()
        ));
    }
    for path in bad {
        let response = get(&server, &path).await;
        assert!(response.starts_with(b"HTTP/1.1 404"), "{path}");
        assert!(!String::from_utf8_lossy(&response).contains("Private page"));
    }
    let check = base
        .join(&format!("{}/pages/start.html", server.check_prefix()))
        .unwrap();
    assert!(
        get(&server, check.path())
            .await
            .starts_with(b"HTTP/1.1 200")
    );
    std::fs::remove_dir_all(root).unwrap();
}
