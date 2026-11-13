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
        "Chrome · Work was removed; no action ran. Open the session again so the user can pick another profile, or ask the user to reconnect this profile."
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

/// Chrome's own record of the extension in the proven folder, with the given `disable_reasons`.
fn extension_record(folder: &Path, reasons: &serde_json::Value) {
    let id = super::setup::EXTENSION_ID;
    std::fs::write(
        folder.join("Preferences"),
        json!({"extensions":{"settings":{id:{"disable_reasons":reasons}}}}).to_string(),
    )
    .unwrap();
}

// PREDICT (red before the change): every offline Chrome/Edge row read only "Offline", whether the extension was
// turned off in that profile or the profile simply had no window open, so the person could not tell what to do. The
// reason now comes from Chrome's own record in that row's proven folder; with no proven folder or no readable record
// the row stays a plain "Offline".
#[tokio::test]
async fn an_offline_row_says_whether_the_extension_is_off_or_no_window_is_open() {
    let core = fixture("presence-offline-reason", json!([])).await;
    let (_data, folder) = browser_data(&core, true);
    status(&core.shared, true);
    status(&core.shared, false);
    assert_eq!(
        row(&core).unwrap().reason,
        None,
        "no record read: plain Offline"
    );
    extension_record(&folder, &json!([1]));
    let off = row(&core).unwrap();
    assert_eq!(off.state, super::ProfileState::Offline);
    assert_eq!(off.reason.as_deref(), Some("Extension off in this profile"));
    extension_record(&folder, &json!([]));
    assert_eq!(
        row(&core).unwrap().reason.as_deref(),
        Some("No window of this profile open")
    );
    let tools = std::mem::take(&mut *core.shared.chrome_control.owner.tools.lock().unwrap());
    assert_eq!(
        row(&core).unwrap().reason,
        None,
        "helper down: a window would not connect either"
    );
    *core.shared.chrome_control.owner.tools.lock().unwrap() = tools;
    status(&core.shared, true);
    assert_eq!(
        row(&core).unwrap().reason,
        None,
        "a connected row has no reason"
    );
    core.shutdown().await;
}

// Chrome writes its preferences some seconds after the person turns the extension off, when the profile is already
// offline and its row was published as "No window of this profile open". The next helper status must publish the new
// reason (the status path compares the whole browser view), and an unchanged one publishes nothing.
#[tokio::test]
async fn a_later_preferences_write_republishes_the_offline_reason() {
    let core = fixture("presence-offline-republish", json!([])).await;
    let (_data, folder) = browser_data(&core, true);
    extension_record(&folder, &json!([]));
    status(&core.shared, true);
    status(&core.shared, false);
    status(&core.shared, false);
    let mut events = core.shared.events.subscribe();
    extension_record(&folder, &json!([1]));
    status(&core.shared, false);
    let mut published = None;
    while let Ok(event) = events.try_recv() {
        if let crate::CoreEvent::ChromeControl { view } = event {
            published = view.paired_profiles.and_then(|rows| {
                rows.into_iter()
                    .find(|row| row.profile_id == ID)
                    .and_then(|row| row.reason)
            });
        }
    }
    assert_eq!(published.as_deref(), Some("Extension off in this profile"));
    status(&core.shared, false);
    assert!(
        std::iter::from_fn(|| events.try_recv().ok())
            .all(|event| !matches!(event, crate::CoreEvent::ChromeControl { .. })),
        "an unchanged reason publishes nothing"
    );
    core.shutdown().await;
}

// PREDICT (red before the change): a paired Chrome whose profiles are all offline shows the reconnecting card, which
// always said "Turn on in Chrome … click Re-enable", also when Chrome only shows its profile picker and the extension
// is on in every paired profile. Once the browser's preferences would record an extension the person turned off
// (15 s), a proven paired folder that says "on" means no window is open: the card says to open one, no Re-enable.
#[tokio::test]
async fn the_card_asks_for_a_window_not_re_enable_when_the_extension_is_on() {
    let core = fixture("presence-card-window", json!([])).await;
    let (_data, folder) = browser_data(&core, true);
    extension_record(&folder, &json!([]));
    status(&core.shared, true);
    status(&core.shared, false);
    let chrome = &core.shared.chrome_control;
    let card = || super::commands::addon(&core.shared);
    assert!(
        !super::profile_reason::no_window(chrome, super::agent_browser::Browser::Chrome),
        "inside the preferences delay nothing is known yet"
    );
    super::profile_reason::offline_for_test(
        chrome,
        super::agent_browser::Browser::Chrome,
        super::profile_reason::PREFERENCES_DELAY,
    );
    assert!(super::profile_reason::no_window(
        chrome,
        super::agent_browser::Browser::Chrome
    ));
    assert_eq!(
        card().state,
        super::AddonState::ChromeClosed,
        "a closed Chrome keeps its own card"
    );
    let mut view = super::addon::view(super::AddonState::Connecting, "");
    super::addon::reconnecting(&mut view, true);
    super::addon::window_closed_for(&mut view, super::agent_browser::Browser::Chrome);
    assert_eq!(view.label, "Open a Chrome window");
    assert!(
        !view.instruction.contains("Re-enable"),
        "{}",
        view.instruction
    );
    assert!(
        !view
            .step
            .as_deref()
            .unwrap_or_default()
            .contains("Re-enable")
    );
    extension_record(&folder, &json!([1]));
    assert!(
        !super::profile_reason::no_window(chrome, super::agent_browser::Browser::Chrome),
        "an extension that is off keeps the Turn on guidance"
    );
    status(&core.shared, true);
    assert!(!super::profile_reason::no_window(
        chrome,
        super::agent_browser::Browser::Chrome
    ));
    core.shutdown().await;
}

// PREDICT (red before the change): macOS never reads a Chrome profile folder, so no folder could say the extension is
// on, and a paired Chrome with no connected profile kept "Turn on in Chrome … click Re-enable" for good, also when
// the extension was on and only waited for approval or had no window. After the preferences delay, with no folder
// readable, the card names those two steps instead; Re-enable stays for an extension a folder shows to be off.
#[tokio::test]
async fn without_readable_profile_folders_the_card_asks_for_a_window_or_approval_not_re_enable() {
    let core =
        super::edge_agent_tests::fixture_with("presence-card-unreadable", json!([]), |config| {
            config.macos = Some(true);
        })
        .await;
    let (_data, folder) = browser_data(&core, true);
    extension_record(&folder, &json!([]));
    status(&core.shared, true);
    status(&core.shared, false);
    let chrome_browser = super::agent_browser::Browser::Chrome;
    let card = || {
        let mut view = super::addon::view(super::AddonState::Connecting, "");
        super::profile_reason::reconnecting(&core.shared, &mut view, true, chrome_browser);
        view
    };
    assert_eq!(
        card().label,
        "Turn on in Chrome",
        "inside the preferences delay"
    );
    super::profile_reason::offline_for_test(
        &core.shared.chrome_control,
        chrome_browser,
        super::profile_reason::PREFERENCES_DELAY,
    );
    let view = card();
    assert_eq!(
        view.step.as_deref(),
        Some("Open a window of a paired Chrome profile, or approve it in Vibe Work.")
    );
    assert_eq!(view.label, "Open a Chrome window");
    assert!(
        !view.instruction.contains("Re-enable"),
        "{}",
        view.instruction
    );
    core.shutdown().await;
}
