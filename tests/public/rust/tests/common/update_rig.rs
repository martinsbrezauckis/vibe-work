//! Rig for the update-flow tests (the fixture): a scripted backend and hooks, a throwaway minisign key per test
//! (`Key`, signing the way the Tauri CLI does), and a temp data folder that is removed
//! when the last run using it ends (`Rig::restart` keeps it). Used by `vw_update_flow.rs` and `vw_update_restart.rs`.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use vw_host::update_flow::{
    BoxFuture, InstallMode, Offer, Phase, Progress, UpdateBackend, UpdateConfig, UpdateHooks,
    UpdateManager, UpdateNotice,
};
use vw_host::update_os::OsTooOld;

pub const CURRENT: &str = "1.0.0";

/// A throwaway key pair; `pubkey` and `sign` use the encodings the Tauri CLI writes.
pub struct Key {
    pub pk: minisign::PublicKey,
    pub sk: minisign::SecretKey,
}

impl Key {
    pub fn new() -> Self {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        Self {
            pk: pair.pk,
            sk: pair.sk,
        }
    }
    pub fn pubkey(&self) -> String {
        STANDARD.encode(self.pk.to_box().unwrap().to_string())
    }
    /// Signature (base64 of the minisig text) bound to `version` like `tauri signer sign --app-version`.
    pub fn sign(&self, data: &[u8], version: &str) -> String {
        let comment = format!("timestamp:1\tfile:app\tversion:{version}");
        let signature = minisign::sign(
            Some(&self.pk),
            &self.sk,
            Cursor::new(data),
            Some(&comment),
            Some("test"),
        )
        .unwrap();
        STANDARD.encode(signature.to_string())
    }
}

#[derive(Default)]
pub struct Script {
    pub check: Mutex<Option<Result<Option<Offer>, String>>>,
    pub package: Mutex<Vec<u8>>,
    pub install_error: Mutex<Option<String>>,
    pub checks: AtomicU32,
    pub downloads: AtomicU32,
    pub installs: Mutex<Vec<(String, Vec<u8>)>>,
    /// The manager to read the view from while a download runs (progress test).
    pub observe: Mutex<Option<UpdateManager>>,
    pub seen: Mutex<Vec<Phase>>,
    /// When set, `download` waits on it (busy test).
    pub gate: Mutex<Option<Arc<tokio::sync::Notify>>>,
}

pub struct FakeBackend(pub Arc<Script>);

impl UpdateBackend for FakeBackend {
    fn check(&self) -> BoxFuture<Result<Option<Offer>, String>> {
        let script = self.0.clone();
        Box::pin(async move {
            script.checks.fetch_add(1, Ordering::SeqCst);
            script
                .check
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(Err("server unreachable".to_owned()))
        })
    }
    fn download(&self, _offer: Offer, progress: Progress) -> BoxFuture<Result<Vec<u8>, String>> {
        let script = self.0.clone();
        Box::pin(async move {
            script.downloads.fetch_add(1, Ordering::SeqCst);
            let gate = script.gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            let bytes = script.package.lock().unwrap().clone();
            progress(bytes.len() as u64, Some(bytes.len() as u64));
            if let Some(manager) = script.observe.lock().unwrap().clone() {
                script.seen.lock().unwrap().push(manager.view().phase);
            }
            Ok(bytes)
        })
    }
    fn install(&self, offer: Offer, package: Vec<u8>) -> BoxFuture<Result<(), String>> {
        let script = self.0.clone();
        Box::pin(async move {
            if let Some(error) = script.install_error.lock().unwrap().clone() {
                return Err(error);
            }
            script
                .installs
                .lock()
                .unwrap()
                .push((offer.version, package));
            Ok(())
        })
    }
}

#[derive(Default)]
pub struct Hooks {
    pub notices: Mutex<Vec<UpdateNotice>>,
    pub restarts: AtomicU32,
}

impl UpdateHooks for Hooks {
    fn notify(&self, notice: UpdateNotice) {
        self.notices.lock().unwrap().push(notice);
    }
    fn restart(&self) -> BoxFuture<()> {
        self.restarts.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {})
    }
}

/// Deletes the data folder when the last run using it is gone.
pub struct DirGuard(pub PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Rig {
    pub dir: PathBuf,
    pub guard: Arc<DirGuard>,
    pub key: Arc<Key>,
    pub script: Arc<Script>,
    pub hooks: Arc<Hooks>,
    pub manager: UpdateManager,
}

pub fn unique_dir(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vw-update-flow-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

impl Rig {
    pub fn new(name: &str) -> Self {
        Self::with(name, InstallMode::InApp, None, None)
    }

    /// `settings`: initial `settings.json` text; `again`: reuse a data dir and key (a restart).
    pub fn with(
        name: &str,
        mode: InstallMode,
        settings: Option<&str>,
        again: Option<(Arc<DirGuard>, Arc<Key>)>,
    ) -> Self {
        let (guard, key) =
            again.unwrap_or_else(|| (Arc::new(DirGuard(unique_dir(name))), Arc::new(Key::new())));
        let dir = guard.0.clone();
        if let Some(text) = settings {
            std::fs::write(dir.join("settings.json"), text).unwrap();
        }
        let script = Arc::new(Script::default());
        let hooks = Arc::new(Hooks::default());
        let manager = UpdateManager::new(
            UpdateConfig {
                current: CURRENT.to_owned(),
                pubkey: key.pubkey(),
                mode,
                data_dir: dir.clone(),
            },
            Arc::new(FakeBackend(script.clone())),
            hooks.clone(),
        );
        Self {
            dir,
            guard,
            key,
            script,
            hooks,
            manager,
        }
    }

    /// A new run on the same data folder and key (an app restart).
    pub fn restart(self, mode: InstallMode) -> Self {
        Self::with("again", mode, None, Some((self.guard, self.key)))
    }

    /// Announces `version` with `package` signed by `signer` for `signed_for`.
    pub fn announce_signed(&self, version: &str, package: &[u8], signer: &Key, signed_for: &str) {
        *self.script.check.lock().unwrap() = Some(Ok(Some(Offer {
            version: version.to_owned(),
            notes: Some("Faster start.".to_owned()),
            signature: signer.sign(package, signed_for),
            os_too_old: None,
        })));
        *self.script.package.lock().unwrap() = package.to_vec();
    }

    pub fn announce(&self, version: &str, package: &[u8]) {
        self.announce_signed(version, package, &self.key, version);
    }

    /// Marks the announced release as needing a newer system than this computer has (the backend's finding).
    pub fn require_os(&self, need: OsTooOld) {
        if let Some(Ok(Some(offer))) = self.script.check.lock().unwrap().as_mut() {
            offer.os_too_old = Some(need);
        }
    }

    pub fn phase(&self) -> Phase {
        self.manager.view().phase
    }

    pub fn installs(&self) -> usize {
        self.script.installs.lock().unwrap().len()
    }

    pub fn notices(&self) -> Vec<UpdateNotice> {
        self.hooks.notices.lock().unwrap().clone()
    }

    pub fn package_file(&self) -> PathBuf {
        self.dir.join("updates/package.bin")
    }
}

pub const PACKAGE: &[u8] = b"pretend this is the 1.1.0 installer";

pub fn is_ready(phase: &Phase, wanted: &str) -> bool {
    matches!(phase, Phase::Ready { version, .. } if version == wanted)
}
