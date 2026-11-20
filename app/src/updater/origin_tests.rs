//! Package address policy of the real updater backend: the address an update announcement names for the package,
//! and every redirect hop of the download, must be one of the channel's package origins before any request is sent.
//!
//! PREDICT (written before the policy existed): the backend hands the announced URL straight to the plugin's
//! download, which sends a GET to whatever the unsigned `latest.json` names and follows every redirect; the
//! signature is checked only after the body arrived. So each refused case below fails red: the recording server
//! (a loopback "package" endpoint, also used as the HTTP proxy so that a request to any host lands there) receives a
//! request, and the error is the plugin's network or signature error instead of the plain refusal. With the policy:
//! zero requests reach it, the error is the plain refusal, the flow's phase is a failed download with that text, and
//! the allowed shapes still go out (the official release address reaches the network; a test channel's own loopback
//! origin downloads, also through a redirect inside that origin).
//!
//! Depends on Tauri's mock runtime and the real updater plugin (not on Windows: see `selection_tests.rs`).

// The mock runtime cannot start in a Windows test program (`selection_tests.rs`).
#![cfg(not(windows))]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::json;
use tauri::test::MockRuntime;
use vw_host::update_flow::{
    BoxFuture, InstallMode, Phase, Stage, UpdateBackend, UpdateConfig, UpdateHooks, UpdateManager,
    UpdateNotice,
};

use vw_host::update_origin::{PackageOrigins, REFUSED};

use super::PluginBackend;

/// The plain failure the person sees (the detail line under "The update could not be downloaded.").
const REFUSED_TEXT: &str =
    "the update address is not Vibe Work's release server, so nothing was downloaded";

/// A loopback HTTP server that records the first line of every request it gets (and every connection, before it
/// reads anything) and answers with `respond(first line)`.
struct Recorder {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Recorder {
    fn start(respond: impl Fn(&str) -> String + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                log.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push("connection".to_owned());
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => request.extend_from_slice(&buffer[..count]),
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let line = text.lines().next().unwrap_or_default().to_owned();
                log.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(line.clone());
                let _ = stream.write_all(respond(&line).as_bytes());
            }
        });
        Self { addr, seen }
    }

    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn seen(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn reply(status: &str, extra: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// A channel server: `/latest.json` announces 99.0.0 with `package` as its package address; other paths answer
/// with `other(first line)`.
fn channel_server(
    package: Arc<Mutex<String>>,
    other: impl Fn(&str) -> String + Send + 'static,
) -> Recorder {
    Recorder::start(move |line| {
        if line.starts_with("GET /latest.json ") {
            let url = package
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            let body = json!({"version": "99.0.0", "url": url, "signature": "c2ln"}).to_string();
            reply("200 OK", "Content-Type: application/json\r\n", &body)
        } else {
            other(line)
        }
    })
}

/// A mock app whose update channel is the test channel at `channel` (loopback http, the insecure-transport flag).
fn app_for(channel: &Recorder) -> tauri::App<MockRuntime> {
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().plugins.0.insert(
        "updater".into(),
        json!({
            "pubkey": "dGVzdA==",
            "endpoints": [format!("{}/latest.json", channel.base())],
            "dangerousInsecureTransportProtocol": true,
            "requireSignedVersion": true,
        }),
    );
    tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap()
}

/// The production backend with the official release hosts as its package origins.
fn release_backend(app: &tauri::App<MockRuntime>) -> PluginBackend<MockRuntime> {
    PluginBackend {
        app: app.handle().clone(),
        cached: Arc::new(Mutex::new(None)),
        origins: Arc::new(PackageOrigins::release()),
    }
}

/// The production backend with the origins the app's own (test) channel config gives, as `start` builds it.
fn channel_backend(app: &tauri::App<MockRuntime>) -> PluginBackend<MockRuntime> {
    PluginBackend {
        app: app.handle().clone(),
        cached: Arc::new(Mutex::new(None)),
        origins: Arc::new(PackageOrigins::for_channel(
            app.config().plugins.0.get("updater"),
        )),
    }
}

/// Checks, then downloads the announced package with every request of the download sent through `proxy`
/// (when given), so a request to any host is visible there.
async fn check_and_download(
    backend: &PluginBackend<MockRuntime>,
    proxy: Option<&Recorder>,
) -> Result<Vec<u8>, String> {
    let offer = backend.check().await.unwrap().expect("an offer");
    if let Some(proxy) = proxy {
        let mut cached = backend
            .cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let update = cached.as_mut().unwrap();
        update.proxy = Some(proxy.base().parse().unwrap());
        update.timeout = Some(Duration::from_secs(5));
    }
    backend.download(offer, Arc::new(|_, _| {})).await
}

#[tokio::test]
async fn a_package_outside_the_release_hosts_gets_no_request() {
    let sink = Recorder::start(|_| reply("200 OK", "", "not a package"));
    let sink_port = sink.addr.port();
    let refused = [
        format!("http://127.0.0.1:{sink_port}/pkg"),
        format!("http://localhost:{sink_port}/pkg"),
        "http://github.com/martinsbrezauckis/vibe-work/releases/download/v99.0.0/pkg".to_owned(),
        "https://140.82.121.4/martinsbrezauckis/vibe-work/releases/download/v99.0.0/pkg".to_owned(),
        "https://[::1]/pkg".to_owned(),
        "https://172.16.0.10/pkg".to_owned(),
        "https://github.com:8443/martinsbrezauckis/vibe-work/releases/download/v99.0.0/pkg"
            .to_owned(),
        "https://user:secret@github.com/martinsbrezauckis/vibe-work/releases/download/v99.0.0/pkg"
            .to_owned(),
        "https://github.com/someone-else/vibe-work/releases/download/v99.0.0/pkg".to_owned(),
        "https://example.org/pkg".to_owned(),
        "https://github.com.example.org/pkg".to_owned(),
    ];
    let package = Arc::new(Mutex::new(String::new()));
    let channel = channel_server(package.clone(), |_| reply("404 Not Found", "", ""));
    let app = app_for(&channel);
    for url in refused {
        package
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone_from(&url);
        let backend = release_backend(&app);
        let result = check_and_download(&backend, Some(&sink)).await;
        assert!(sink.seen().is_empty(), "{url}: {:?}", sink.seen());
        assert_eq!(
            result.as_ref().err().map(String::as_str),
            Some(REFUSED_TEXT),
            "{url}"
        );
    }
}

#[tokio::test]
async fn a_redirect_out_of_the_channel_stops_before_the_next_request() {
    let sink = Recorder::start(|_| reply("200 OK", "", "not a package"));
    let target = format!("{}/pkg", sink.base());
    let package = Arc::new(Mutex::new(String::new()));
    let channel = channel_server(package.clone(), move |_| {
        reply("302 Found", &format!("Location: {target}\r\n"), "")
    });
    package
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone_from(&format!("{}/first", channel.base()));
    let app = app_for(&channel);
    let backend = channel_backend(&app);
    let result = check_and_download(&backend, None).await;
    assert!(sink.seen().is_empty(), "{:?}", sink.seen());
    assert_eq!(
        result.as_ref().err().map(String::as_str),
        Some(REFUSED_TEXT)
    );
    assert!(
        channel
            .seen()
            .iter()
            .any(|line| line.starts_with("GET /first ")),
        "{:?}",
        channel.seen()
    );
}

#[tokio::test]
async fn the_flow_shows_the_refusal_as_a_failed_download() {
    let sink = Recorder::start(|_| reply("200 OK", "", "not a package"));
    let package = Arc::new(Mutex::new(format!("{}/pkg", sink.base())));
    let channel = channel_server(package, |_| reply("404 Not Found", "", ""));
    let app = app_for(&channel);
    let data = tempfile::tempdir().unwrap();
    let manager = UpdateManager::new(
        UpdateConfig {
            current: "0.0.1".to_owned(),
            pubkey: "dGVzdA==".to_owned(),
            mode: InstallMode::InApp,
            data_dir: data.path().to_path_buf(),
        },
        Arc::new(release_backend(&app)),
        Arc::new(NoHooks),
    );
    let _ = manager.check().await;
    assert!(sink.seen().is_empty(), "{:?}", sink.seen());
    match manager.view().phase {
        Phase::Failed {
            stage: Stage::Download,
            error,
            ..
        } => assert_eq!(error, REFUSED_TEXT),
        other => panic!("{other:?}"),
    }
    assert_eq!(REFUSED, REFUSED_TEXT);
}

#[tokio::test]
async fn the_official_release_address_still_reaches_the_network() {
    // The recording proxy stands in for the network: an https request through it starts with CONNECT to the host.
    let proxy = Recorder::start(|_| reply("502 Bad Gateway", "", ""));
    let package = Arc::new(Mutex::new(
        "https://github.com/martinsbrezauckis/vibe-work/releases/download/v99.0.0/Vibe-Work_99.0.0_amd64.AppImage"
            .to_owned(),
    ));
    let channel = channel_server(package, |_| reply("404 Not Found", "", ""));
    let app = app_for(&channel);
    let backend = release_backend(&app);
    let result = check_and_download(&backend, Some(&proxy)).await;
    assert_ne!(
        result.as_ref().err().map(String::as_str),
        Some(REFUSED_TEXT)
    );
    assert!(
        proxy
            .seen()
            .iter()
            .any(|line| line.starts_with("CONNECT github.com:443 ")),
        "{:?}",
        proxy.seen()
    );
}

#[tokio::test]
async fn a_test_channel_downloads_from_its_own_origin_also_through_a_redirect() {
    let package = Arc::new(Mutex::new(String::new()));
    let channel = channel_server(package.clone(), |line| {
        if line.starts_with("GET /first ") {
            reply("302 Found", "Location: /package\r\n", "")
        } else {
            reply("200 OK", "", "package bytes")
        }
    });
    package
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone_from(&format!("{}/first", channel.base()));
    let app = app_for(&channel);
    let backend = channel_backend(&app);
    let result = check_and_download(&backend, None).await;
    // The bytes arrived and reached the signature check (the fixture is not signed).
    let error = result.expect_err("an unsigned fixture package fails its signature check");
    assert_ne!(error, REFUSED_TEXT);
    let seen = channel.seen();
    assert!(
        seen.iter().any(|line| line.starts_with("GET /package ")),
        "{seen:?}"
    );
}

struct NoHooks;

impl UpdateHooks for NoHooks {
    fn notify(&self, _notice: UpdateNotice) {}
    fn restart(&self) -> BoxFuture<()> {
        Box::pin(async {})
    }
}

/// A refused channel's reason reaches Settings; a build without a channel or with an accepted one has none.
/// PREDICT (written before the change): did not compile (no `refusal`): the refusal was only logged.
#[test]
fn a_refused_channel_keeps_its_reason_for_settings() {
    let refused = serde_json::json!({"pubkey": "k", "requireSignedVersion": true,
        "endpoints": ["https://example.invalid/latest.json"]});
    let reason = super::refusal(Some(&refused))
        .0
        .expect("a refused channel has a reason");
    assert_eq!(
        reason,
        "this copy's update address is not Vibe Work's release server"
    );
    // Another refusal keeps its own text.
    let unsigned = serde_json::json!({"pubkey": "k", "requireSignedVersion": false,
        "endpoints": ["https://github.com/martinsbrezauckis/vibe-work/releases/latest/download/latest.json"]});
    assert_eq!(
        super::refusal(Some(&unsigned)).0.as_deref(),
        Some("update channel refused: requireSignedVersion must be true")
    );
    let accepted = serde_json::json!({"pubkey": "k", "requireSignedVersion": true,
        "endpoints": ["https://github.com/martinsbrezauckis/vibe-work/releases/latest/download/latest.json"]});
    assert_eq!(super::refusal(Some(&accepted)).0, None);
    assert_eq!(super::refusal(None).0, None);
}
