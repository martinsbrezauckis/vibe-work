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
    *chrome.owner.pairing.lock().unwrap() = ("Paired".into(), None);
    chrome.owner.answering(answer)
}

/// Feed a complete helper status through the same observer used by the owner task.
#[cfg(feature = "your-edge")]
pub(crate) fn observe_status(shared: &Arc<Shared>, data: &Value) {
    super::owner::update_pairing(&Arc::downgrade(shared), &json!({"structuredContent":data}));
}
