//! Offline media capture routing regressions: the core's guard, a recording proxy and a fake preview engine.
//! No public socket or desktop browser is used.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use vw_browser::capture::Fetch;
use vw_netguard::upstream::{Error as ProxyError, ProxyResolver};
use vw_netguard::{NetGuard, Resolve, ResolveFuture, Upstream};
use vw_preview::{
    ConsoleEntry, EngineFuture, HistoryStep, MountId, MountSpec, OpenRequest, PageEvent, Placement,
    PreviewConfig, PreviewEngine, PreviewHooks, Storage,
};

use super::{CoreFetch, pinned_direct_addresses};

#[test]
fn private_page_public_answers_use_proxy_and_mixed_answers_stay_local() {
    let public: SocketAddr = "93.184.216.34:443".parse().unwrap();
    let private: SocketAddr = "10.20.30.40:443".parse().unwrap();
    assert_eq!(
        pinned_direct_addresses(vw_preview::Destination::Public, Some(&[public])),
        None
    );
    assert_eq!(
        pinned_direct_addresses(vw_preview::Destination::Public, Some(&[public, private])),
        Some(vec![private]),
    );
}

#[derive(Default)]
struct Engine {
    hooks: Mutex<Option<Arc<dyn PreviewHooks>>>,
    mounted: Mutex<Option<MountId>>,
}
impl PreviewEngine for Engine {
    fn attach(&self, hooks: Arc<dyn PreviewHooks>) {
        *self.hooks.lock().unwrap() = Some(hooks);
    }
    fn mount(&self, spec: MountSpec) -> EngineFuture<'_, ()> {
        *self.mounted.lock().unwrap() = Some(spec.id);
        Box::pin(async { Ok(()) })
    }
    fn navigate(&self, _: &MountId, _: url::Url) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn history(&self, _: &MountId, _: HistoryStep) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn place(&self, _: &MountId, _: Option<Placement>) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn close(&self, _: &MountId) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn screenshot(&self, _: &MountId) -> EngineFuture<'_, Vec<u8>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn drain_console(&self, _: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn wipe(&self, _: &Storage) -> EngineFuture<'_, Vec<PathBuf>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

struct Dns;
impl Resolve for Dns {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        let ip: IpAddr = if host == "private.test" {
            "10.20.30.40".parse().unwrap()
        } else {
            "93.184.216.34".parse().unwrap()
        };
        Box::pin(async move { Ok(vec![SocketAddr::new(ip, port)]) })
    }
}

struct Native {
    endpoint: String,
    calls: AtomicUsize,
}
impl ProxyResolver for Native {
    fn resolve(&self, _: &str) -> Result<Vec<String>, ProxyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![self.endpoint.clone()])
    }
}

async fn fixture(
    name: &str,
    page: &str,
    local_port: Option<u16>,
) -> (crate::Core, NetGuard, CoreFetch, Arc<Engine>) {
    let guard = NetGuard::bind(Arc::new(Dns)).unwrap();
    let owned = guard.clone();
    let engine = Arc::new(Engine::default());
    let owned_engine = engine.clone();
    let core = crate::offline_core::start_with(name, move |config, dir| {
        config.preview = Some(crate::previews::PreviewSetup {
            engine: owned_engine,
            config: PreviewConfig::new(dir.join("preview")),
            guard: Some(owned),
        });
    })
    .await;
    let service = core.shared.preview.as_ref().unwrap().service.clone();
    if let Some(port) = local_port {
        service.set_chat_servers(1, vec![port], Arc::new(move |asked| asked == port));
    }
    service
        .open(OpenRequest {
            session_id: "capture-page".into(),
            url: page.parse().unwrap(),
            allowed_ports: local_port.into_iter().collect(),
            downloads_dir: None,
            wait: false,
            lan: false,
        })
        .await
        .unwrap();
    let fetch = CoreFetch::new(&core.shared, service).unwrap();
    (core, guard, fetch, engine)
}

async fn head(stream: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(4), async {
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(stream.read_u8().await.unwrap());
        }
    })
    .await
    .unwrap();
    String::from_utf8(bytes).unwrap()
}

#[tokio::test]
async fn public_media_uses_manual_proxy_for_checked_address() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (core, guard, fetch, _engine) =
        fixture("capture-manual-proxy", "https://93.184.216.34/", None).await;
    guard.set_upstream(Upstream::from_values(
        Some(&format!("http://{}", proxy.local_addr().unwrap())),
        Some(&format!("http://{}", proxy.local_addr().unwrap())),
        None,
        None,
    ));
    let response = tokio::spawn(async move {
        fetch
            .fetch("capture-page", "http://public.test/media.png", 4096)
            .await
    });
    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(4), proxy.accept())
        .await
        .unwrap()
        .unwrap();
    let request = head(&mut stream).await;
    assert!(
        request.starts_with("GET http://93.184.216.34/media.png HTTP/1.1"),
        "{request}"
    );
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 3\r\nConnection: close\r\n\r\nPNG").await.unwrap();
    assert_eq!(response.await.unwrap().unwrap().bytes, b"PNG");
    core.shutdown().await;
}

#[tokio::test]
async fn public_media_asks_system_pac_for_each_request() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let native = Arc::new(Native {
        endpoint: format!("http://{}", proxy.local_addr().unwrap()),
        calls: AtomicUsize::new(0),
    });
    let (core, guard, fetch, _engine) =
        fixture("capture-pac-per-hop", "https://93.184.216.34/", None).await;
    guard.set_upstream(Upstream::none().with_resolver(native.clone()));
    let proxy_task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(4), proxy.accept())
                .await
                .unwrap()
                .unwrap();
            requests.push(head(&mut stream).await);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nPNG")
                .await
                .unwrap();
        }
        requests
    });
    for part in ["one", "two"] {
        let result = fetch
            .fetch(
                "capture-page",
                &format!("http://public.test/{part}.png"),
                4096,
            )
            .await
            .unwrap();
        assert_eq!(result.bytes, b"PNG");
    }
    let requests = proxy_task.await.unwrap();
    assert!(requests[0].starts_with("GET http://93.184.216.34/one.png HTTP/1.1"));
    assert!(requests[1].starts_with("GET http://93.184.216.34/two.png HTTP/1.1"));
    assert_eq!(native.calls.load(Ordering::SeqCst), 2);
    core.shutdown().await;
}

#[tokio::test]
async fn two_chats_fetch_public_media_through_separate_checked_proxy_requests() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (core, guard, fetch, _engine) =
        fixture("capture-two-chats", "https://93.184.216.34/", None).await;
    let service = core.shared.preview.as_ref().unwrap().service.clone();
    service
        .open(OpenRequest {
            session_id: "second-page".into(),
            url: "https://93.184.216.34/".parse().unwrap(),
            allowed_ports: vec![],
            downloads_dir: None,
            wait: false,
            lan: false,
        })
        .await
        .unwrap();
    guard.set_upstream(Upstream::from_values(
        Some(&format!("http://{}", proxy.local_addr().unwrap())),
        None,
        None,
        None,
    ));
    let proxy_task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(4), proxy.accept())
                .await
                .unwrap()
                .unwrap();
            requests.push(head(&mut stream).await);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nPNG")
                .await
                .unwrap();
        }
        requests
    });
    let (first, second) = tokio::join!(
        fetch.fetch("capture-page", "http://public.test/first.png", 4096),
        fetch.fetch("second-page", "http://public.test/second.png", 4096),
    );
    assert_eq!(first.unwrap().bytes, b"PNG");
    assert_eq!(second.unwrap().bytes, b"PNG");
    let requests = proxy_task.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .any(|head| head.starts_with("GET http://93.184.216.34/first.png HTTP/1.1"))
    );
    assert!(
        requests
            .iter()
            .any(|head| head.starts_with("GET http://93.184.216.34/second.png HTTP/1.1"))
    );
    core.shutdown().await;
}

#[tokio::test]
async fn empty_media_url_refuses_without_opening_proxy() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (core, guard, fetch, _engine) =
        fixture("capture-empty-input", "https://93.184.216.34/", None).await;
    guard.set_upstream(Upstream::from_values(
        Some(&format!("http://{}", proxy.local_addr().unwrap())),
        None,
        None,
        None,
    ));
    assert_eq!(
        fetch.fetch("capture-page", "", 4096).await.unwrap_err(),
        "refused: not a URL"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(150), proxy.accept())
            .await
            .is_err()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn https_media_connect_names_the_checked_ip() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (core, guard, fetch, _engine) =
        fixture("capture-https-proxy", "https://93.184.216.34/", None).await;
    guard.set_upstream(Upstream::from_values(
        None,
        Some(&format!("http://{}", proxy.local_addr().unwrap())),
        None,
        None,
    ));
    let response = tokio::spawn(async move {
        fetch
            .fetch("capture-page", "https://public.test/media.png", 4096)
            .await
    });
    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(4), proxy.accept())
        .await
        .unwrap()
        .unwrap();
    let request = head(&mut stream).await;
    assert!(
        request.starts_with("CONNECT 93.184.216.34:443 HTTP/1.1"),
        "{request}"
    );
    stream
        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
        .await
        .unwrap();
    assert!(response.await.unwrap().is_err());
    core.shutdown().await;
}

#[tokio::test]
async fn dead_selected_proxy_refuses_without_a_direct_media_socket() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = destination.local_addr().unwrap().port();
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = unavailable.local_addr().unwrap();
    drop(unavailable);
    let (core, guard, mut fetch, _engine) =
        fixture("capture-dead-proxy", "https://93.184.216.34/", None).await;
    guard.set_upstream(Upstream::from_values(
        Some(&format!("http://{proxy}")),
        None,
        None,
        None,
    ));
    // The stand-in direct client would reach this local listener if the fetcher skips the guard.
    fetch.client = vw_mistral::tls_builder()
        .no_proxy()
        .resolve("public.test", destination.local_addr().unwrap())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let direct = tokio::spawn(async move {
        match tokio::time::timeout(Duration::from_millis(300), destination.accept()).await {
            Ok(Ok((mut stream, _))) => {
                let _ = head(&mut stream).await;
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nPNG",
                    )
                    .await
                    .unwrap();
                true
            }
            _ => false,
        }
    });
    let fetch = Arc::new(fetch);
    let task_fetch = Arc::clone(&fetch);
    let response = tokio::spawn(async move {
        task_fetch
            .fetch(
                "capture-page",
                &format!("http://public.test:{port}/media.png"),
                4096,
            )
            .await
    });
    let reason = response.await.unwrap().unwrap_err();
    assert!(reason.contains("proxy could not be reached"), "{reason}");
    assert_eq!(
        fetch.capture_refusal().as_deref(),
        Some("refused: The proxy could not be reached.")
    );
    assert!(
        !direct.await.unwrap(),
        "a direct socket reached the stand-in public host"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn public_name_with_private_answer_is_refused_before_proxy() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (core, guard, fetch, _engine) =
        fixture("capture-private-dns", "https://93.184.216.34/", None).await;
    guard.set_upstream(Upstream::from_values(
        Some(&format!("http://{}", proxy.local_addr().unwrap())),
        None,
        None,
        None,
    ));
    let reason = fetch
        .fetch("capture-page", "http://private.test/media.png", 4096)
        .await
        .unwrap_err();
    assert!(
        reason.starts_with("refused:") && reason.contains("private network address"),
        "{reason}"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(150), proxy.accept())
            .await
            .is_err()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn public_media_refuses_when_the_guard_is_unavailable() {
    let core = crate::offline_core::start("capture-no-guard").await;
    let service = vw_preview::PreviewService::new(
        Arc::new(Engine::default()),
        PreviewConfig::new(PathBuf::new()),
    );
    service
        .open(OpenRequest {
            session_id: "capture-page".into(),
            url: "https://93.184.216.34/".parse().unwrap(),
            allowed_ports: vec![],
            downloads_dir: None,
            wait: false,
            lan: false,
        })
        .await
        .unwrap();
    let fetch = CoreFetch::new(&core.shared, service).unwrap();
    let reason = fetch
        .fetch("capture-page", "http://public.test/media.png", 4096)
        .await
        .unwrap_err();
    assert_eq!(reason, "refused: the browser network guard is unavailable");
    assert_eq!(fetch.capture_refusal().as_deref(), Some(reason.as_str()));
    core.shutdown().await;
}

#[tokio::test]
async fn proxy_redirect_to_loopback_is_rechecked_before_a_new_socket() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (core, guard, fetch, _engine) =
        fixture("capture-redirect-proxy", "https://93.184.216.34/", None).await;
    guard.set_upstream(Upstream::from_values(
        Some(&format!("http://{}", proxy.local_addr().unwrap())),
        None,
        None,
        None,
    ));
    let response = tokio::spawn(async move {
        fetch
            .fetch("capture-page", "http://public.test/media.png", 4096)
            .await
    });
    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(4), proxy.accept())
        .await
        .unwrap()
        .unwrap();
    let request = head(&mut stream).await;
    assert!(
        request.starts_with("GET http://93.184.216.34/media.png HTTP/1.1"),
        "{request}"
    );
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        destination.local_addr().unwrap().port()
    );
    stream.write_all(redirect.as_bytes()).await.unwrap();
    let reason = response.await.unwrap().unwrap_err();
    assert!(reason.starts_with("refused:"), "{reason}");
    assert!(
        tokio::time::timeout(Duration::from_millis(150), destination.accept())
            .await
            .is_err()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn local_preview_media_remains_direct_when_proxy_is_dead() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = destination.local_addr().unwrap().port();
    let (core, guard, fetch, _engine) = fixture(
        "capture-local-direct",
        &format!("http://127.0.0.1:{port}/"),
        Some(port),
    )
    .await;
    guard.set_upstream(Upstream::from_values(
        Some("http://127.0.0.1:9"),
        None,
        None,
        None,
    ));
    let response = tokio::spawn(async move {
        fetch
            .fetch(
                "capture-page",
                &format!("http://127.0.0.1:{port}/media.png"),
                4096,
            )
            .await
    });
    let accepted = tokio::time::timeout(Duration::from_secs(4), destination.accept()).await;
    let (mut stream, _) = match accepted {
        Ok(Ok(value)) => value,
        other => panic!(
            "local media socket {other:?}; fetch={:?}",
            response.await.unwrap().map(|_| "ok")
        ),
    };
    assert!(
        head(&mut stream)
            .await
            .starts_with("GET /media.png HTTP/1.1")
    );
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nPNG")
        .await
        .unwrap();
    assert_eq!(response.await.unwrap().unwrap().bytes, b"PNG");
    core.shutdown().await;
}

#[tokio::test]
async fn pinned_lan_preview_media_remains_direct_when_proxy_is_dead() {
    let route = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    route.connect("192.0.2.1:1").unwrap();
    let ip = route.local_addr().unwrap().ip();
    assert_eq!(
        vw_netguard::ip::classify(ip),
        vw_netguard::ip::IpClass::Private
    );
    let destination = TcpListener::bind(SocketAddr::new(ip, 0)).await.unwrap();
    let port = destination.local_addr().unwrap().port();
    let page = format!("http://{ip}:{port}/");
    let (core, guard, fetch, engine) =
        fixture("capture-lan-direct", "https://93.184.216.34/", None).await;
    let service = core.shared.preview.as_ref().unwrap().service.clone();
    service
        .open(OpenRequest {
            session_id: "capture-page".into(),
            url: page.parse().unwrap(),
            allowed_ports: vec![],
            downloads_dir: None,
            wait: false,
            lan: true,
        })
        .await
        .unwrap();
    let label = engine
        .mounted
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .label
        .clone();
    service.record_connected_addresses(
        &label,
        &ip.to_string(),
        port,
        &[destination.local_addr().unwrap()],
    );
    assert!(
        engine
            .hooks
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .page_event(&label, PageEvent::Started(page.parse().unwrap()))
    );
    guard.set_upstream(Upstream::from_values(
        Some("http://127.0.0.1:9"),
        None,
        None,
        None,
    ));
    let response = tokio::spawn(async move {
        fetch
            .fetch(
                "capture-page",
                &format!("http://{ip}:{port}/media.png"),
                4096,
            )
            .await
    });
    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(4), destination.accept())
        .await
        .unwrap()
        .unwrap();
    assert!(
        head(&mut stream)
            .await
            .starts_with("GET /media.png HTTP/1.1")
    );
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nPNG")
        .await
        .unwrap();
    assert_eq!(response.await.unwrap().unwrap().bytes, b"PNG");
    core.shutdown().await;
}
