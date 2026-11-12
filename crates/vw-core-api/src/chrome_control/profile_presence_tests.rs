//! Deleted Chrome/Edge profiles: only a folder proven by its own extension storage can show a deletion, and the row
//! leaves Settings only once no chat or pending approval uses it. Uses temporary browser folders, no native browser.
#![allow(
    clippy::unwrap_used,
    reason = "temporary browser folders and fixtures must be created for the test to mean anything"
)]
use super::{edge_agent_tests::fixture, test_support};
use serde_json::json;
use std::path::{Path, PathBuf};

const UUID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
const ID: &str = "chrome:1b4e28ba-2fa1-41d2-883f-0016d3cca427";

/// Where the extension's private `IndexedDB` keeps the UUID.
#[derive(Clone, Copy)]
enum Layout {
    /// Chromium 156: one SQLite file per database under `IndexedDB/chrome-extension_<id>_0/`.
    Sqlite,
    /// Earlier releases: `IndexedDB/chrome-extension_<id>_0.indexeddb.leveldb/`.
    LevelDb,
}

/// A Chrome user data folder registered by Set up, with one `Work` profile folder that has the extension.
fn browser_data(core: &crate::Core, uuid_in_storage: bool) -> (PathBuf, PathBuf) {
    browser_data_in(core, uuid_in_storage, Layout::Sqlite)
}

fn browser_data_in(
    core: &crate::Core,
    uuid_in_storage: bool,
    layout: Layout,
) -> (PathBuf, PathBuf) {
    let chrome = &core.shared.chrome_control;
    let data = chrome
        .config
        .user_home
        .as_ref()
        .unwrap()
        .join("presence-browser-data");
    let folder = data.join("Profile 3");
    let id = &chrome.config.extension_id;
    std::fs::create_dir_all(folder.join("Extensions").join(id).join("0.1.7_0")).unwrap();
    let value = if uuid_in_storage { UUID } else { "another" };
    let origin = format!("chrome-extension_{id}_0");
    // V8 serializes the one-byte string as plain bytes in either layout (the SQLite case was checked with Chromium 156).
    let (storage, file, bytes) = match layout {
        Layout::Sqlite => (
            folder.join("IndexedDB").join(&origin),
            "WCHYFF2I4HCHMUPPZAC67CSVYI6DH5O5Q6INA6VYGXVHAVFYHNPQ",
            format!("SQLite format 3\0\u{ff}\u{14}\"${value}"),
        ),
        Layout::LevelDb => (
            folder
                .join("IndexedDB")
                .join(format!("{origin}.indexeddb.leveldb")),
            "000003.log",
            format!("\u{1}\u{1}\tprofileId\u{ff}\u{14}\"${value}"),
        ),
    };
    std::fs::create_dir_all(&storage).unwrap();
    std::fs::write(storage.join(file), bytes).unwrap();
    local_state(&data, &["Profile 3"]);
    std::fs::create_dir_all(chrome.dir().unwrap()).unwrap();
    std::fs::write(
        chrome.dir().unwrap().join("setup.json"),
        json!({"files":[],"browsers":["Google Chrome"],"userData":[data]}).to_string(),
    )
    .unwrap();
    (data, folder)
}

fn local_state(data: &Path, folders: &[&str]) {
    let cache: serde_json::Map<_, _> = folders
        .iter()
        .map(|name| ((*name).to_owned(), json!({"name":"Work"})))
        .collect();
    std::fs::write(
        data.join("Local State"),
        json!({"profile":{"info_cache":cache}}).to_string(),
    )
    .unwrap();
}

fn status(shared: &std::sync::Arc<crate::core::Shared>, connected: bool) {
    test_support::observe_status(
        shared,
        &json!({"profiles":[{"profileId":ID,"name":"Work","connected":connected}]}),
    );
}

/// Chrome deleted the profile: its folder is gone and `Local State` no longer lists it.
fn delete_profile(data: &Path, folder: &Path) {
    std::fs::remove_dir_all(folder).unwrap();
    local_state(data, &[]);
}

/// The agent tools are what open per-call browser approval cards.
async fn tools_fixture(name: &str) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        config.agent_tools = Some(crate::AgentToolsConfig::new(
            dir.join("tools"),
            dir.join("data"),
            None,
        ));
    })
    .await;
    test_support::chat(&core.shared, "s");
    core
}

fn row(core: &crate::Core) -> Option<super::BrowserProfile> {
    super::profiles::view(&core.shared)
        .into_iter()
        .find(|row| row.profile_id == ID)
}

#[tokio::test]
async fn a_deleted_profile_proven_by_its_extension_storage_leaves_settings_when_unused() {
    deleted_and_unused("presence-deleted-unused", Layout::Sqlite).await;
    deleted_and_unused("presence-deleted-unused-leveldb", Layout::LevelDb).await;
}

async fn deleted_and_unused(name: &str, layout: Layout) {
    let core = fixture(name, json!([])).await;
    let (data, folder) = browser_data_in(&core, true, layout);
    status(&core.shared, true);
    status(&core.shared, false);
    assert_eq!(row(&core).unwrap().state, super::ProfileState::Offline);
    delete_profile(&data, &folder);
    status(&core.shared, false);
    assert_eq!(
        row(&core),
        None,
        "an unused deleted profile leaves Settings"
    );
    // The helper still lists its stale pairing as offline; the retired row must not come back.
    status(&core.shared, false);
    assert_eq!(row(&core), None, "a later status does not bring it back");
    let saved = super::profiles::Profiles::load(core.shared.chrome_control.dir());
    assert!(
        saved
            .records
            .get(ID)
            .is_some_and(|record| record.removed && record.deleted),
        "the saved inventory keeps it retired across a restart"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_chat_bound_to_a_deleted_profile_keeps_the_row_and_names_the_next_step() {
    let core = fixture("presence-deleted-bound", json!([])).await;
    let chrome = &core.shared.chrome_control;
    test_support::chat(&core.shared, "chat-a");
    let (data, folder) = browser_data(&core, true);
    status(&core.shared, true);
    super::profile_bindings::remember(chrome, "chat-a", ID, None).unwrap();
    status(&core.shared, false);
    delete_profile(&data, &folder);
    status(&core.shared, false);

    let kept = row(&core).expect("a bound profile is never removed");
    assert_eq!(kept.state, super::ProfileState::Unavailable);
    assert!(!kept.eligible);
    assert_eq!(kept.reason.as_deref(), Some("Removed in Chrome"));
    let error = super::profiles::check(chrome, ID).unwrap_err();
    assert_eq!(
        error,
        "Chrome · Work was removed; no action ran. Choose another profile in this chat or reconnect this profile."
    );
    let chat = super::profile_selection::view(chrome).unwrap();
    assert_eq!(chat[0].state, "removed");
    assert_eq!(chat[0].name, "Chrome · Work");

    super::end_chat(&core.shared, "chat-a", true);
    assert_eq!(
        row(&core),
        None,
        "the row leaves once the last chat is gone"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn offline_profiles_without_deletion_evidence_stay_in_settings() {
    // No association: the folder's storage does not hold this UUID, so its later absence proves nothing.
    let core = fixture("presence-unproven", json!([])).await;
    let (data, folder) = browser_data(&core, false);
    status(&core.shared, true);
    status(&core.shared, false);
    delete_profile(&data, &folder);
    status(&core.shared, false);
    let kept = row(&core).expect("an unproven absence stays");
    assert_eq!(kept.state, super::ProfileState::Offline);
    assert_eq!(kept.reason, None);
    core.shutdown().await;

    // Proven folder, but Chrome still lists it in Local State: not a deletion yet.
    let core = fixture("presence-still-listed", json!([])).await;
    let (data, folder) = browser_data(&core, true);
    status(&core.shared, true);
    status(&core.shared, false);
    std::fs::remove_dir_all(&folder).unwrap();
    local_state(&data, &["Profile 3"]);
    status(&core.shared, false);
    assert_eq!(row(&core).unwrap().state, super::ProfileState::Offline);
    core.shutdown().await;
}

#[tokio::test]
async fn a_pending_browser_approval_keeps_a_deleted_profile_until_it_settles() {
    let core = tools_fixture("presence-deleted-pending").await;
    let (data, folder) = browser_data(&core, true);
    status(&core.shared, true);
    let held = super::profile_presence::hold_for_test(&core.shared, ID);
    assert!(held.is_some(), "the test card opened");
    status(&core.shared, false);
    delete_profile(&data, &folder);
    status(&core.shared, false);
    assert!(row(&core).is_some(), "a pending approval keeps the row");
    drop(held);
    status(&core.shared, false);
    assert_eq!(row(&core), None);
    core.shutdown().await;
}
