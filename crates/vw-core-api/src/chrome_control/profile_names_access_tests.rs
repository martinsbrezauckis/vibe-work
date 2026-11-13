//! macOS: when the system refused the profile-name search a browser's user data folder, that browser's card says how
//! to allow it (`ChromeSetupView::profile_names_refused`), a button opens System Settings at Files & Folders, and a
//! recheck after the person comes back reads the folder again, so the names appear without a restart.
//!
//! PREDICT (written before the change, against stubbed request ops and a view field that is never set): every test
//! that expects the fact or a recheck fails. The view never reports a refusal (`profile_names_refused` stays `None`)
//! because nothing maps the gate's per-run refusal memory to a browser; a recheck reads nothing (the refusal is
//! remembered until the app restarts), so the names stay "Chrome profile 1" after the person allowed the read; the
//! settings button opens nothing. The tests that expect no fact (allowed folder, other OS) or no read (recheck with
//! nothing refused) pass on the stub as well and guard the "never elsewhere" and "no extra reads" rules.
//!
//! Uses temporary browser folders laid out like Chrome's and a switchable recording permission boundary.
#![allow(
    clippy::unwrap_used,
    reason = "temporary browser folders and fixtures must be created for the test to mean anything"
)]
use super::commands::ChromeControlRequest;
use super::profile_names_tests::{HOME, HOME_UUID, WORK, WORK_UUID, browser_data, names, observe};
use crate::Origin;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

struct Rig {
    core: crate::Core,
    /// Reads attempted inside the browser's user data folder.
    calls: Arc<Mutex<Vec<PathBuf>>>,
    /// While set, the permission boundary refuses those reads, as macOS does before the person allows them.
    refuse: Arc<AtomicBool>,
    /// Addresses the System Settings button opened.
    opened: Arc<Mutex<Vec<String>>>,
}

async fn rig(name: &str, macos: bool, refuse: bool, opener_fails_first: bool) -> Rig {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let refusing = Arc::new(AtomicBool::new(refuse));
    let opened = Arc::new(Mutex::new(Vec::new()));
    let (seen, refused, log) = (calls.clone(), refusing.clone(), opened.clone());
    let core = super::edge_agent_tests::fixture_with(name, json!([]), move |config| {
        config.macos = Some(macos);
        config.file_access = Some(Arc::new(move |_, path: &Path| {
            if path
                .components()
                .any(|part| part.as_os_str() == "names-browser-data")
            {
                seen.lock().unwrap().push(path.to_owned());
                if refused.load(Ordering::SeqCst) {
                    return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
                }
            }
            Ok(())
        }));
        config.settings_opener = Some(Arc::new(move |url: &str| {
            let mut log = log.lock().unwrap();
            log.push(url.to_owned());
            if opener_fails_first && log.len() == 1 {
                return Err("no such pane".into());
            }
            Ok(())
        }));
    })
    .await;
    Rig {
        core,
        calls,
        refuse: refusing,
        opened,
    }
}

fn refused(core: &crate::Core) -> (Option<bool>, Option<bool>) {
    let view = super::commands::view(&core.shared);
    (
        view.setup.profile_names_refused,
        view.edge.and_then(|edge| edge.profile_names_refused),
    )
}

async fn recheck(core: &crate::Core) -> super::commands::ChromeControlView {
    match core
        .chrome_control(Origin::Ui, ChromeControlRequest::RecheckProfileNames {})
        .await
        .unwrap()
    {
        crate::CommandResult::ChromeControl { view, .. } => view,
        other => panic!("{other:?}"),
    }
}

fn seed(core: &crate::Core) {
    browser_data(
        core,
        &[("Profile 3", WORK_UUID), ("Profile 1", HOME_UUID)],
        &json!({"Profile 3":{"name":"Work"},"Profile 1":{"name":"Personal"}}),
    );
}

#[tokio::test]
async fn a_macos_refusal_shows_the_allow_line_in_that_browsers_card_only() {
    let rig = rig("names-access-refused", true, true, false).await;
    seed(&rig.core);
    observe(&rig.core, &[WORK]);
    assert_eq!(names(&rig.core)[0].1, "Chrome profile 1");
    // Chrome's card shows the line; Edge's card (no refusal, no paired Edge profile) does not.
    assert_eq!(refused(&rig.core), (Some(true), None));
    rig.core.shutdown().await;
}

#[tokio::test]
async fn no_allow_line_when_the_folder_is_readable_or_off_macos() {
    let allowed = rig("names-access-allowed", true, false, false).await;
    seed(&allowed.core);
    observe(&allowed.core, &[WORK]);
    assert_eq!(names(&allowed.core)[0].1, "Work");
    assert_eq!(refused(&allowed.core), (None, None));
    allowed.core.shutdown().await;

    // Windows and Linux never check the folder, so a refusing boundary changes nothing and shows nothing.
    let other = rig("names-access-other-os", false, true, false).await;
    seed(&other.core);
    observe(&other.core, &[WORK]);
    assert_eq!(refused(&other.core), (None, None));
    other.core.shutdown().await;
}

#[tokio::test]
async fn a_removed_profile_takes_the_allow_line_with_it() {
    let rig = rig("names-access-removed", true, true, false).await;
    seed(&rig.core);
    observe(&rig.core, &[WORK]);
    assert_eq!(refused(&rig.core).0, Some(true));
    // Remove in Settings leaves a `removed` record.
    rig.core
        .shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .records
        .get_mut(WORK)
        .unwrap()
        .removed = true;
    assert_eq!(refused(&rig.core).0, None);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn allowing_it_later_shows_the_names_after_a_recheck_without_a_restart() {
    let rig = rig("names-access-allow-later", true, true, false).await;
    seed(&rig.core);
    observe(&rig.core, &[WORK, HOME]);
    let labels: Vec<_> = names(&rig.core).into_iter().map(|row| row.1).collect();
    assert_eq!(labels, ["Chrome profile 1", "Chrome profile 2"]);
    // The person turns on Files & Folders > Vibe Work > Google Chrome and comes back to the window.
    rig.refuse.store(false, Ordering::SeqCst);
    let view = recheck(&rig.core).await;
    assert_eq!(view.setup.profile_names_refused, None);
    let mut labels: Vec<_> = names(&rig.core).into_iter().map(|row| row.1).collect();
    labels.sort();
    assert_eq!(labels, ["Personal", "Work"]);
    let shown: Vec<_> = view
        .paired_profiles
        .unwrap_or_default()
        .into_iter()
        .map(|row| row.name)
        .collect();
    assert!(shown.contains(&"Work".to_owned()), "{shown:?}");
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_recheck_while_still_refused_tries_once_and_keeps_the_line() {
    let rig = rig("names-access-still-refused", true, true, false).await;
    seed(&rig.core);
    observe(&rig.core, &[WORK]);
    let before = rig.calls.lock().unwrap().len();
    assert_eq!(before, 1);
    let view = recheck(&rig.core).await;
    assert_eq!(view.setup.profile_names_refused, Some(true));
    assert_eq!(rig.calls.lock().unwrap().len(), before + 1);
    // Status updates in between do not try again: only a recheck (the person came back) does.
    observe(&rig.core, &[WORK]);
    assert_eq!(rig.calls.lock().unwrap().len(), before + 1);
    assert_eq!(names(&rig.core)[0].1, "Chrome profile 1");
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_recheck_with_nothing_refused_reads_nothing() {
    let rig = rig("names-access-nothing", true, false, false).await;
    seed(&rig.core);
    observe(&rig.core, &[WORK]);
    let before = rig.calls.lock().unwrap().len();
    recheck(&rig.core).await;
    assert_eq!(rig.calls.lock().unwrap().len(), before);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn the_button_opens_files_and_folders_on_macos_only() {
    let mac = rig("names-access-open", true, true, false).await;
    mac.core
        .chrome_control(Origin::Ui, ChromeControlRequest::OpenProfileNameSettings {})
        .await
        .unwrap();
    assert_eq!(
        *mac.opened.lock().unwrap(),
        ["x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders"]
    );
    mac.core.shutdown().await;

    // When that pane cannot be opened, Privacy & Security itself.
    let fallback = rig("names-access-open-fallback", true, true, true).await;
    fallback
        .core
        .chrome_control(Origin::Ui, ChromeControlRequest::OpenProfileNameSettings {})
        .await
        .unwrap();
    assert_eq!(
        *fallback.opened.lock().unwrap(),
        [
            "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders",
            "x-apple.systempreferences:com.apple.preference.security"
        ]
    );
    fallback.core.shutdown().await;

    let other = rig("names-access-open-other-os", false, true, false).await;
    assert!(
        other
            .core
            .chrome_control(Origin::Ui, ChromeControlRequest::OpenProfileNameSettings {})
            .await
            .is_err()
    );
    assert!(other.opened.lock().unwrap().is_empty());
    other.core.shutdown().await;
}
