//! Synthetic release assets and update-flow helper shared by manifest test binaries.
//! Inputs are disposable files and throwaway signatures supplied by each caller.

use std::path::Path;
use std::sync::Arc;

use crate::common::update_rig::{DirGuard, FakeBackend, Hooks, Script, is_ready, unique_dir};
use serde_json::Value;
use vw_host::update_flow::{InstallMode, Offer, UpdateConfig, UpdateManager};

pub fn names(version: &str) -> [String; 3] {
    [
        format!("Vibe-Work_{version}_x64-setup.exe"),
        format!("Vibe-Work_{version}_aarch64.app.tar.gz"),
        format!("Vibe-Work_{version}_amd64.AppImage"),
    ]
}

pub fn content(name: &str) -> Vec<u8> {
    let magic: &[u8] = if name.ends_with(".exe") {
        b"MZ"
    } else if name.ends_with(".tar.gz") {
        &[0x1f, 0x8b]
    } else {
        b"\x7fELF"
    };
    [magic, format!(" pretend this is {name}").as_bytes()].concat()
}

pub async fn flow_from_manifest(
    manifest: &Path,
    assets: &Path,
    pubkey: &str,
    tamper: bool,
    version: &str,
) -> (bool, bool) {
    let json: Value = serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    let entry = &json["platforms"]["linux-x86_64"];
    let url = entry["url"].as_str().unwrap();
    let name = url.rsplit('/').next().unwrap();
    let mut bytes = std::fs::read(assets.join(name)).unwrap();
    if tamper {
        bytes[3] ^= 0x01;
    }
    let script = Arc::new(Script::default());
    *script.check.lock().unwrap() = Some(Ok(Some(Offer {
        version: json["version"].as_str().unwrap().to_owned(),
        notes: json["notes"].as_str().map(str::to_owned),
        signature: entry["signature"].as_str().unwrap().to_owned(),
    })));
    *script.package.lock().unwrap() = bytes;
    let dir = unique_dir("manifest-flow");
    let _guard = DirGuard(dir.clone());
    let manager = UpdateManager::new(
        UpdateConfig {
            current: "1.0.0".to_owned(),
            pubkey: pubkey.to_owned(),
            mode: InstallMode::InApp,
            data_dir: dir.clone(),
        },
        Arc::new(FakeBackend(script)),
        Arc::new(Hooks::default()),
    );
    let _ = manager.check().await;
    let ready = is_ready(&manager.view().phase, version);
    let kept = dir.join("updates/package.bin").exists();
    (ready, kept)
}
