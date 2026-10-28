//! Rig for the Vibe update tests (the fixture): a local fake release channel (loopback HTTP server holding
//! `vibe-support.json`, its `.sig`, the requirements files and the uv archive, counting requests per file), manifest
//! and signature builders (a throwaway key per test, signing the way `tools/public/vibe-support/sign` does), and an
//! installer that starts with Vibe Work's own install of the rig's version already in place. No network.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::Path as UrlPath;
use axum::http::StatusCode;
use axum::routing::get;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use vw_vibe_install::{Limits, SupportConfig};

use super::update_rig::Key;
use super::vibe_rig::{Rig, VIBE, archive_kind, rig, sources, uv_archive};

/// The app version the evaluation runs as.
pub const APP: &str = "0.1.0";

type Files = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// The fake channel.
pub struct Channel {
    pub base: String,
    files: Files,
    hits: Arc<Mutex<HashMap<String, u32>>>,
    _task: tokio::task::JoinHandle<()>,
}

impl Channel {
    pub async fn start() -> Self {
        let files: Files = Arc::default();
        let hits: Arc<Mutex<HashMap<String, u32>>> = Arc::default();
        let (served, counted) = (files.clone(), hits.clone());
        let app = Router::new().route(
            "/{name}",
            get(move |UrlPath(name): UrlPath<String>| {
                *counted.lock().unwrap().entry(name.clone()).or_default() += 1;
                let body = served.lock().unwrap().get(&name).cloned();
                async move {
                    match body {
                        Some(bytes) => (StatusCode::OK, bytes),
                        None => (StatusCode::NOT_FOUND, Vec::new()),
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            base: format!("http://127.0.0.1:{port}"),
            files,
            hits,
            _task: task,
        }
    }

    pub fn url(&self, name: &str) -> String {
        format!("{}/{name}", self.base)
    }

    pub fn put(&self, name: &str, bytes: Vec<u8>) {
        self.files.lock().unwrap().insert(name.to_owned(), bytes);
    }

    pub fn remove(&self, name: &str) {
        self.files.lock().unwrap().remove(name);
    }

    pub fn hits(&self, name: &str) -> u32 {
        self.hits.lock().unwrap().get(name).copied().unwrap_or(0)
    }

    /// Publishes `manifest` with a signature by `key` for `vibe-support.json`.
    pub fn publish(&self, key: &Key, manifest: &[u8]) {
        self.put("vibe-support.json", manifest.to_vec());
        self.put(
            "vibe-support.json.sig",
            sign_as(key, manifest, "vibe-support.json").into_bytes(),
        );
    }
}

/// A signature of `data` (base64 of the minisign text) whose trusted comment names `file`, as the release signer
/// writes it.
pub fn sign_as(key: &Key, data: &[u8], file: &str) -> String {
    let comment = format!("timestamp:1\tfile:{file}");
    let signature = minisign::sign(
        Some(&key.pk),
        &key.sk,
        Cursor::new(data),
        Some(&comment),
        Some("test"),
    )
    .unwrap();
    STANDARD.encode(signature.to_string())
}

/// The requirements file of a real Vibe `version` (the fake uv reads the version from it).
pub fn requirements_for(version: &str) -> Vec<u8> {
    format!("mistral-vibe=={version} --hash=sha256:00\n").into_bytes()
}

/// One manifest entry for `version`, tested with Vibe Work `min`..=`max`, with a wheel for this OS.
pub fn entry(version: &str, min: &str, max: Option<&str>) -> Value {
    let requirements = requirements_for(version);
    json!({
        "version": version,
        "python": "3.12",
        "minVibeWork": min,
        "maxVibeWork": max,
        "requirements": format!("vibe-{version}.requirements.txt"),
        "requirementsSha256": vw_vibe_install::pin::sha256_hex(&requirements),
        "requirementsBytes": requirements.len(),
        "packages": 101,
        "wheels": {
            vw_vibe_install::this_target(): {
                "filename": format!("mistral_vibe-{version}-fake.whl"),
                "bytes": 1,
                "sha256": "11".repeat(32),
            }
        }
    })
}

/// A manifest's bytes.
pub fn manifest(sequence: u64, pypi_latest: Option<&str>, entries: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "schema": 1,
        "sequence": sequence,
        "published": "2026-10-11T12:00:00Z",
        "pypiLatest": pypi_latest,
        "versions": entries,
    }))
    .unwrap()
}

/// Puts every listed version's requirements file on the channel (what the release step uploads).
pub fn upload_requirements(channel: &Channel, versions: &[&str]) {
    for version in versions {
        channel.put(
            &format!("vibe-{version}.requirements.txt"),
            requirements_for(version),
        );
    }
}

/// Lays out a synthetic install of `version` under `root` (an inert server-path placeholder; no process is started).
pub fn install_marker(root: &Path, version: &str) {
    let layout = vw_vibe_install::layout::Layout::new(root.to_path_buf(), version).unwrap();
    std::fs::create_dir_all(layout.server().parent().unwrap()).unwrap();
    std::fs::write(layout.server(), b"install-marker fixture: never executed\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(layout.server(), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let record = vw_vibe_install::layout::Installed {
        version: version.to_owned(),
        server: layout.server(),
        uv: "0.12.24".to_owned(),
        requirements_sha256: String::new(),
        disk_bytes: 1,
        install_ms: 1,
        finished_at: 1,
    };
    std::fs::write(layout.marker(), serde_json::to_string(&record).unwrap()).unwrap();
}

/// An installer running Vibe Work's own install of the rig's version (`VIBE`), whose uv archive is on `channel`.
pub fn running_rig(dir: &Path, channel: &Channel) -> Rig {
    install_marker(&dir.join("data/vibe"), VIBE);
    let archive = uv_archive();
    let name = format!("uv.{}", archive_kind());
    channel.put(&name, archive.clone());
    rig(
        dir,
        sources(&channel.url(&name), &archive),
        "ok",
        Limits::default(),
        &[],
    )
}

/// The check's config for `channel`, verified with `key`.
pub fn support(channel: &Channel, key: &Key) -> SupportConfig {
    SupportConfig {
        manifest_url: channel.url("vibe-support.json"),
        pubkey: key.pubkey(),
        app_version: APP.to_owned(),
        target: vw_vibe_install::this_target(),
    }
}
