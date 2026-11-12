//! A connected Chrome fixture for gateway tests: app-owned setup plus a live fake helper.
//! All files stay under the offline core's isolated package/home; tests may then change individual facts.
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub(crate) fn answering(
    shared: &Shared,
    answer: impl Fn(&str, &Value) -> Value + Send + 'static,
) -> Arc<Mutex<Vec<(String, Value)>>> {
    let chrome = &shared.chrome_control;
    if let Some(package) = &chrome.config.package {
        let bin = package.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(super::setup::helper_name()), b"test helper").unwrap();
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
