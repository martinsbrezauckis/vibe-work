//! Rig for the Vibe install tests (the fixture): a uv release archive that holds the fake uv (`.tar.gz` on Unix,
//! `.zip` on Windows, laid out like Astral's), a loopback download server that counts requests, `Sources` that
//! point at it with the archive's real SHA-256, and an installer over a scratch root whose steps run the fake uv
//! with its settings in `extra_env`. Used by `vw_vibe_install.rs` and `vw_vibe_install_debug_api.rs`.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use vw_vibe_install::{InstallView, InstallerConfig, Limits, Sources, UvAsset, VibeInstaller};

use super::closed_port::ClosedPort;

/// The fake Vibe version the rig installs (fake_uv reports it). Deliberately independent of the real pin
/// (`crates/vw-vibe-install/pin`), so a Vibe bump never touches the install-flow tests.
pub const VIBE: &str = "2.26.0";

pub fn fake_uv() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake_uv"))
}

pub fn archive_kind() -> &'static str {
    if cfg!(windows) { "zip" } else { "tar.gz" }
}

/// Astral's layout: `uv-<target>/uv` + `uvx` in a tar.gz; `uv.exe` + `uvx.exe` at the top of a zip.
pub fn uv_archive() -> Vec<u8> {
    let uv = std::fs::read(fake_uv()).unwrap();
    if cfg!(windows) {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        let options = zip::write::SimpleFileOptions::default();
        for name in ["uv.exe", "uvx.exe"] {
            zip.start_file(name, options).unwrap();
            zip.write_all(&uv).unwrap();
        }
        zip.finish().unwrap();
        out.into_inner()
    } else {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        for name in ["uv", "uvx"] {
            let mut header = tar::Header::new_gnu();
            header.set_size(uv.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(
                &mut header,
                format!("uv-x86_64-unknown-linux-gnu/{name}"),
                uv.as_slice(),
            )
            .unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }
}

/// A loopback server for one archive at `/uv.<kind>`.
pub struct Index {
    pub url: String,
    /// `http://127.0.0.1:<port>` (for the redirect routes).
    pub base: String,
    pub hits: Arc<AtomicU32>,
    _task: tokio::task::JoinHandle<()>,
}

pub async fn serve(bytes: Vec<u8>) -> Index {
    let hits = Arc::new(AtomicU32::new(0));
    let counted = hits.clone();
    let bytes = Arc::new(bytes);
    let path = format!("/uv.{}", archive_kind());
    let target = path.clone();
    // `/hop/<n>` redirects n times (relative Location, like a CDN hop) before the archive; `/away` redirects to a
    // plain-http address that is not loopback (must be refused without connecting).
    let app = Router::new()
        .route(
            &path,
            get(move || {
                let bytes = bytes.clone();
                counted.fetch_add(1, Ordering::SeqCst);
                async move { bytes.as_ref().clone() }
            }),
        )
        .route(
            "/hop/{n}",
            get(move |axum::extract::Path(n): axum::extract::Path<u32>| {
                let next = if n == 0 {
                    target.clone()
                } else {
                    format!("/hop/{}", n - 1)
                };
                async move {
                    (
                        axum::http::StatusCode::FOUND,
                        [(axum::http::header::LOCATION, next)],
                    )
                }
            }),
        )
        .route(
            "/away",
            get(|| async {
                (
                    axum::http::StatusCode::FOUND,
                    [(axum::http::header::LOCATION, "http://example.invalid/uv")],
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Index {
        base: format!("http://127.0.0.1:{port}"),
        url: format!("http://127.0.0.1:{port}{path}"),
        hits,
        _task: task,
    }
}

/// An archive URL and its reservation. The caller keeps the guard until every download attempt has finished.
pub fn closed_url() -> (String, ClosedPort) {
    let closed = ClosedPort::bind();
    let url = closed.url(&format!("/uv.{}", archive_kind()));
    (url, closed)
}

pub fn sources(url: &str, archive: &[u8]) -> Sources {
    let requirements = format!("mistral-vibe=={VIBE} --hash=sha256:00\n");
    Sources {
        vibe_version: VIBE.to_owned(),
        python: "3.12".to_owned(),
        uv_version: "0.12.24".to_owned(),
        uv: UvAsset {
            url: url.to_owned(),
            bytes: archive.len() as u64,
            sha256: vw_vibe_install::pin::sha256_hex(archive),
            archive: archive_kind().to_owned(),
        },
        uv_release_page: "https://github.com/astral-sh/uv/releases/tag/0.12.24".to_owned(),
        pypi_page: format!("https://pypi.org/project/mistral-vibe/{VIBE}/"),
        docs: "https://github.com/mistralai/mistral-vibe#readme".to_owned(),
        requirements_sha256: vw_vibe_install::pin::sha256_hex(requirements.as_bytes()),
        requirements,
        packages: 101,
        wheel: (format!("mistral_vibe-{VIBE}-fake.whl"), 1, "11".repeat(32)),
        index_url: "https://pypi.org/simple".to_owned(),
    }
}

/// The installer of one test; `ready` records every server the `on_ready` hook got.
pub struct Rig {
    pub dir: PathBuf,
    pub installer: VibeInstaller,
    pub log: PathBuf,
    pub ready: Arc<Mutex<Vec<PathBuf>>>,
}

/// `mode` is the fake uv's `FAKE_UV_MODE`; `start_error` makes the `on_ready` hook fail; `path_dirs` is the
/// `PATH` the existing-install discovery searches. The parent environment holds redirections a user might have
/// (`UV_INDEX_URL`, `PIP_INDEX_URL`, `UV_CONFIG_FILE`): the steps must not see them.
pub fn rig(dir: &Path, sources: Sources, mode: &str, limits: Limits, path_dirs: &[PathBuf]) -> Rig {
    rig_with(dir, Ok(sources), mode, limits, path_dirs, None)
}

pub fn rig_with(
    dir: &Path,
    sources: Result<Sources, vw_vibe_install::PinError>,
    mode: &str,
    limits: Limits,
    path_dirs: &[PathBuf],
    start_error: Option<String>,
) -> Rig {
    let log = dir.join("fake-uv.jsonl");
    let mut parent_env: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    for (name, value) in [
        ("UV_INDEX_URL", "http://127.0.0.1:9/evil"),
        ("PIP_INDEX_URL", "http://127.0.0.1:9/evil"),
        ("UV_CONFIG_FILE", "/nonexistent/uv.toml"),
    ] {
        parent_env.push((name.into(), value.into()));
    }
    let installer = VibeInstaller::new(InstallerConfig {
        root: dir.join("data").join("vibe"),
        sources,
        parent_env,
        extra_env: vec![
            ("FAKE_UV_LOG".to_owned(), log.clone().into_os_string()),
            (
                "FAKE_UV_SERVER".to_owned(),
                super::fake::fake_bin().into_os_string(),
            ),
            ("FAKE_UV_VIBE_VERSION".to_owned(), VIBE.into()),
            ("FAKE_UV_MODE".to_owned(), mode.into()),
        ],
        discovery: vw_supervisor::Discovery {
            explicit: None,
            path_var: Some(std::env::join_paths(path_dirs).unwrap()),
            home: None,
        },
        limits,
    });
    let ready = Arc::new(Mutex::new(Vec::new()));
    let seen = ready.clone();
    installer.set_on_ready(Arc::new(move |server| {
        seen.lock().unwrap().push(server);
        let result = start_error.clone().map_or(Ok(()), Err);
        Box::pin(async move { result })
    }));
    Rig {
        dir: dir.to_path_buf(),
        installer,
        log,
        ready,
    }
}

/// The installer over `<dir>/data/vibe`, with the fake server installed there (as the setup install leaves it)
/// when `installed`.
pub fn pinned_installer(dir: &Path, installed: bool) -> VibeInstaller {
    let root = dir.join("data/vibe");
    let version = vw_vibe_install::Sources::pinned_version().unwrap();
    let layout = vw_vibe_install::layout::Layout::new(root.clone(), &version).unwrap();
    if installed {
        std::fs::create_dir_all(layout.server().parent().unwrap()).unwrap();
        std::fs::copy(super::fake::fake_bin(), layout.server()).unwrap();
        let record = vw_vibe_install::layout::Installed {
            version,
            server: layout.server(),
            uv: "0.12.24".to_owned(),
            requirements_sha256: String::new(),
            disk_bytes: 1,
            install_ms: 1,
            finished_at: 1,
        };
        std::fs::write(layout.marker(), serde_json::to_string(&record).unwrap()).unwrap();
    }
    vw_vibe_install::VibeInstaller::new(vw_vibe_install::InstallerConfig {
        root,
        sources: vw_vibe_install::Sources::pinned(&vw_vibe_install::this_target()),
        parent_env: Vec::new(),
        extra_env: Vec::new(),
        discovery: vw_supervisor::Discovery::default(),
        limits: vw_vibe_install::Limits::default(),
    })
}

/// Waits until the view's phase is `state` (or the time is up) and returns the last view.
pub async fn until(installer: &VibeInstaller, state: &str, limit: Duration) -> InstallView {
    let mut views = installer.subscribe();
    let end = tokio::time::Instant::now() + limit;
    loop {
        let view = views.borrow_and_update().clone();
        if phase_name(&view) == state {
            return view;
        }
        if tokio::time::timeout_at(end, views.changed()).await.is_err() {
            return view;
        }
    }
}

pub fn phase_name(view: &InstallView) -> String {
    serde_json::to_value(&view.phase).unwrap()["state"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The fake uv's recorded calls. A step killed for its time limit (v7) can die in the middle of writing its line,
/// so an unfinished last line (no trailing newline) is skipped; every complete line must parse.
pub fn uv_calls(log: &Path) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let complete = match text.rfind('\n') {
        Some(end) => &text[..=end],
        None => "",
    };
    complete
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// Whether a process with `pid` is alive and not a zombie (`ps` state; Windows: `tasklist`). A zombie still
/// answers `kill -0`, so existence alone would call a killed process alive.
pub fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let out = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        !state.is_empty() && !state.starts_with('Z')
    }
    #[cfg(windows)]
    {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    }
}
