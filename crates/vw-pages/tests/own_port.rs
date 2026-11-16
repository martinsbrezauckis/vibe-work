//! Every page server (Project tab pages, the files viewer, a one-document snapshot) is a listener of Vibe Work itself:
//! from the moment it is bound until it is dropped, its port is in the browser guard's own ports and in the content
//! rules, so a web page is refused at once ("belongs to Vibe Work itself") and the person is never asked about it.
//!
//! PREDICT (before page servers bound through `vw_netguard::listeners::bind_own_tokio`): the page server binds its
//! own `TcpListener` and nothing registers its port, so `NetGuard::internal_ports` and
//! `rules::current_inputs().internal_ports` lack it; a web page's request to it takes the Local Network Access route
//! and raises the ask.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;
use vw_netguard::{NetGuard, SystemResolver, rules};
use vw_pages::PagesServer;

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("vw-own-port-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/start.html"), "<title>Start</title>").unwrap();
    root
}

fn held(guard: &NetGuard, port: u16) -> bool {
    guard.internal_ports().contains(&port) && rules::current_inputs().internal_ports.contains(&port)
}

#[tokio::test]
async fn every_page_server_port_is_vibe_works_own_until_the_server_is_dropped() {
    let guard = NetGuard::bind(Arc::new(SystemResolver)).unwrap();
    let root = fixture();
    let pages = PagesServer::start(root.clone(), true, "Private".into())
        .await
        .unwrap();
    let files = PagesServer::start_for_files(root.clone(), "Private".into())
        .await
        .unwrap();
    let (document, _url) = PagesServer::start_document("note.html", b"<p>note</p>".to_vec())
        .await
        .unwrap();
    let ports = [pages.port(), files.port(), document.port()];
    for port in ports {
        assert!(
            held(&guard, port),
            "page server port {port} is not Vibe Work's own"
        );
    }
    drop((pages, files, document));
    // A dropped server's task ends at the runtime's next turn; its port leaves the set with the listener.
    let released = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while ports
            .iter()
            .any(|port| rules::current_inputs().internal_ports.contains(port))
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        released.is_ok(),
        "page server ports still held after the servers closed"
    );
    guard.shutdown();
    std::fs::remove_dir_all(root).unwrap();
}
