//! Chrome and Edge profile names on macOS, where a browser's user data folder is another app's protected data.
//!
//! PREDICT (written before the change): the first two tests fail on the base. On macOS
//! `probe::profiles_for` returns no folder before any file call (the protected-app-data boundary), so
//! `profile_presence` never proves a folder and the names stay "Chrome profile 1/2" (first test), and the folder is
//! never even tried, so the refusal test sees no attempt at all. After the change the name search reads the
//! browser's folders on macOS too; a refusal by the system is remembered for that folder and the folder is not read
//! again in this run, because each refused read can raise another macOS alert.
//!
//! Uses temporary browser folders laid out like Chrome's and a recording permission boundary, no native browser.
#![allow(
    clippy::unwrap_used,
    reason = "temporary browser folders and fixtures must be created for the test to mean anything"
)]
use super::profile_names_tests::{HOME, HOME_UUID, WORK, WORK_UUID, browser_data, names, observe};
use serde_json::json;
use std::path::{Path, PathBuf};

/// A Core that behaves as on macOS; `refuse` makes the recording permission boundary refuse every read inside the
/// browser's user data folder, as macOS does for protected app data the person did not allow.
async fn macos_core(
    name: &str,
    refuse: bool,
) -> (crate::Core, std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>) {
    let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = calls.clone();
    let core = super::edge_agent_tests::fixture_with(name, json!([]), move |config| {
        config.macos = Some(true);
        config.file_access = Some(std::sync::Arc::new(move |_, path: &Path| {
            if path
                .components()
                .any(|part| part.as_os_str() == "names-browser-data")
            {
                seen.lock().unwrap().push(path.to_owned());
                if refuse {
                    return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
                }
            }
            Ok(())
        }));
    })
    .await;
    (core, calls)
}

#[tokio::test]
async fn macos_shows_the_browsers_own_profile_names_too() {
    let (core, _) = macos_core("names-macos", false).await;
    browser_data(
        &core,
        &[("Profile 3", WORK_UUID), ("Profile 1", HOME_UUID)],
        &json!({
            "Profile 3":{"name":"Work","user_name":"work@example.com"},
            "Profile 1":{"name":"Personal","user_name":""}
        }),
    );
    observe(&core, &[WORK, HOME]);
    let rows = names(&core);
    assert!(
        rows.contains(&(WORK.into(), "Work".into(), Some("work@example.com".into()))),
        "{rows:?}"
    );
    assert!(
        rows.contains(&(HOME.into(), "Personal".into(), None)),
        "{rows:?}"
    );
    // The account stays on the screen only; agent text adds its domain.
    let mut result = json!({"structuredContent":{},"content":[]});
    super::profile_agent::status(&core.shared, "s", &mut result);
    let text = result.to_string();
    assert!(
        text.contains("Work (example.com)") && !text.contains('@'),
        "{text}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_macos_refusal_keeps_numbered_names_and_is_not_asked_again() {
    let (core, calls) = macos_core("names-macos-refused", true).await;
    browser_data(
        &core,
        &[("Profile 3", WORK_UUID), ("Profile 1", HOME_UUID)],
        &json!({"Profile 3":{"name":"Work"},"Profile 1":{"name":"Personal"}}),
    );
    observe(&core, &[WORK]);
    observe(&core, &[WORK, HOME]);
    let rows = names(&core);
    let labels: Vec<_> = rows.iter().map(|row| row.1.as_str()).collect();
    assert_eq!(labels, ["Chrome profile 1", "Chrome profile 2"], "{rows:?}");
    let first = calls.lock().unwrap().len();
    assert_eq!(
        first,
        1,
        "one attempt, refused: {:?}",
        calls.lock().unwrap()
    );
    // Later searches (the quick retries, then once a minute) never touch the refused folder again.
    for _ in 0..3 {
        core.shared.chrome_control.profiles.lock().unwrap().scanned = None;
        super::profile_presence::reconcile(&core.shared);
    }
    assert_eq!(
        calls.lock().unwrap().len(),
        first,
        "{:?}",
        calls.lock().unwrap()
    );
    core.shutdown().await;
}

/// The real read, not only the injected boundary: a folder the operating system refuses (here a folder without
/// permissions) is refused once and stays refused for this run even after it becomes readable.
#[cfg(unix)]
#[tokio::test]
async fn a_folder_the_system_refuses_on_macos_is_left_alone_for_this_run() {
    use std::os::unix::fs::PermissionsExt;
    let (core, _) = macos_core("names-macos-real-refusal", false).await;
    let data = browser_data(
        &core,
        &[("Profile 3", WORK_UUID)],
        &json!({"Profile 3":{"name":"Work"}}),
    );
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o000)).unwrap();
    let readable_anyway = std::fs::read_dir(&data).is_ok();
    observe(&core, &[WORK]);
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    if readable_anyway {
        // Running with privileges that ignore folder permissions: nothing to prove here.
        core.shutdown().await;
        return;
    }
    assert_eq!(names(&core)[0].1, "Chrome profile 1");
    core.shared.chrome_control.profiles.lock().unwrap().scanned = None;
    observe(&core, &[WORK]);
    assert_eq!(names(&core)[0].1, "Chrome profile 1");
    core.shutdown().await;
}
