//! A connected Chrome fixture for gateway tests: app-owned setup plus a live fake helper.
//! All files stay under the offline core's isolated package/home; tests may then change individual facts.
use crate::core::Shared;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{Arc, Mutex};

fn install_fake_helper(path: &Path) {
    // Several fixtures call answering after Core startup. Rewriting the same bytes can race
    // Windows holding the helper image open during its start probe.
    if !path.is_file() {
        std::fs::write(path, b"test helper").unwrap();
    }
}

pub(crate) fn answering(
    shared: &Shared,
    answer: impl Fn(&str, &Value) -> Value + Send + 'static,
) -> Arc<Mutex<Vec<(String, Value)>>> {
    let chrome = &shared.chrome_control;
    if let Some(package) = &chrome.config.package {
        let bin = package.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        install_fake_helper(&bin.join(super::setup::helper_name()));
        let dir = chrome.dir().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("setup.json"),
            json!({"files":[], "browsers":["Google Chrome"], "userData":[]}).to_string(),
        )
        .unwrap();
    }
    *chrome.owner.pairing.lock().unwrap() = (crate::chrome_control::PairingState::Paired, None);
    chrome.owner.answering(answer)
}

#[cfg(windows)]
#[test]
fn an_existing_fake_helper_is_not_rewritten_while_windows_has_it_open() {
    use std::os::windows::fs::OpenOptionsExt;

    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join(super::setup::helper_name());
    install_fake_helper(&helper);
    // FILE_SHARE_READ excludes writes, as when CreateProcess holds the helper image.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&helper)
        .unwrap();
    install_fake_helper(&helper);
    drop(held);
}

/// Waits until an approved Safari pairing has run to its end (the approval task sets `safari_approved` last), bounded
/// at 10 s. A fixed short sleep is not enough when a loaded machine runs the approval task later.
#[cfg(feature = "your-safari")]
pub(crate) async fn until_safari_approved(shared: &Shared) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while !shared
        .chrome_control
        .owner
        .safari_approved
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the approved Safari pairing never finished"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Feed a complete helper status through the same observer used by the owner task.
pub(crate) fn observe_status(shared: &Arc<Shared>, data: &Value) {
    super::owner::update_pairing(&Arc::downgrade(shared), &json!({"structuredContent":data}));
}

/// Hold the Firefox availability read while another task publishes a question; no timed filesystem fixture.
#[cfg(feature = "your-firefox")]
pub(crate) fn with_firefox_probe_held(shared: &Shared, held: impl FnOnce()) {
    let _probe = shared.chrome_control.owner.firefox_pairing.lock().unwrap();
    held();
}

/// Create a separate chat for multi-profile isolation tests.
pub(super) fn chat(shared: &Shared, id: &str) {
    let _ = shared
        .store
        .load(
            json!({"session":{"id":id},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    shared.access.observe_request(
        "session/agent/update",
        id,
        &json!({"agentName":"auto-approve"}),
    );
}
