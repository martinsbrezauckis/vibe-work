//! A whole-browser Remove whose profile inventory cannot be saved tells the person.
//!
//! PREDICT (red before the change): `profiles::remove_browser` only logged the failed save and the Remove answered with no
//! hint, so the person never learned that the removed profiles would come back after a restart: the test fails on
//! `hint` being `None`. After: the removal stays in memory (the helper side is already unpaired) and the Remove's
//! hint says that the profiles may show again after a restart.
//!
//! PREDICT (`an_inventory_unreadable_at_start_also_says_the_removal_is_not_saved`, red before the change): an inventory
//! that was already unreadable when Vibe Work started (a folder or an unreadable file in its place) made
//! `remove_browser_saved` answer `Ok` without saving, so the Remove said only "Removed." and the removed profiles came
//! back once the file was readable again: the test fails on `hint` being `None`. After: the unsaved removal answers
//! with the same hint as a failed save.
use super::*;
use vw_platform::sync::LockRecover;

/// A core with Chrome and Edge set up on one shared helper and `chrome:one` paired; the helper answers every Remove.
#[allow(clippy::unwrap_used, reason = "isolated test fixture initialization")]
async fn two_browsers(name: &str) -> (crate::Core, registry_native_tests::TestRoot) {
    let registration = registry_native_tests::TestRoot::new(name);
    let core = crate::offline_core::start_with(name, |config, dir| {
        let package = dir.join("package");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(
            package.join("bin").join(setup::helper_name()),
            b"shared helper",
        )
        .unwrap();
        config.chrome_control.package = Some(package);
        config.chrome_control.user_home = Some(dir.join("home"));
        config.chrome_control.windows_registry_root =
            format!(r"{}\NativeMessagingHosts", registration.key());
        config.chrome_control.edge_registry_root =
            format!(r"{}\Edge\NativeMessagingHosts", registration.key());
    })
    .await;
    let chrome = &core.shared.chrome_control;
    let state = chrome.dir().unwrap().to_path_buf();
    setup::install(&chrome.config, &state, &core.shared.instructions.own_home).unwrap();
    setup::register_edge(&chrome.config, &state).unwrap();
    profiles::paired(chrome, "chrome:one", None);
    assert!(
        state.join("browser-profiles.json").is_file(),
        "the paired profile is saved"
    );
    chrome.owner.answering(|name, _| {
        if name == "owner.browserUnpair" {
            json!({"unpaired":true})
        } else {
            json!({"applied":true})
        }
    });
    (core, registration)
}

/// Settings' Remove for Chrome, answering its hint.
#[allow(clippy::unwrap_used, reason = "the test Remove must answer")]
async fn remove_chrome(core: &crate::Core) -> Option<String> {
    let result = core
        .chrome_control(crate::Origin::Ui, ChromeControlRequest::Remove {})
        .await
        .unwrap();
    let crate::CommandResult::ChromeControl { hint, .. } = result else {
        panic!("a Remove answers with the browser view");
    };
    hint
}

fn not_saved() -> String {
    profiles::REMOVAL_NOT_SAVED.replace("{browser}", "Chrome")
}

#[tokio::test]
async fn a_remove_that_cannot_save_the_profiles_says_so() {
    let (core, _registration) = two_browsers("browser-remove-unsaved").await;
    let chrome = &core.shared.chrome_control;
    let inventory = chrome.dir().unwrap().join("browser-profiles.json");
    // A folder where the inventory file goes: every later save of it fails.
    std::fs::remove_file(&inventory).unwrap();
    std::fs::create_dir(&inventory).unwrap();
    assert_eq!(remove_chrome(&core).await, Some(not_saved()));
    let saved = chrome
        .profiles
        .lock_or_recover()
        .records
        .get("chrome:one")
        .map(|record| record.removed);
    assert_eq!(saved, Some(true), "this run keeps the removal");
    std::fs::remove_dir(&inventory).unwrap();
    core.shutdown().await;
}

#[tokio::test]
async fn an_inventory_unreadable_at_start_also_says_the_removal_is_not_saved() {
    let (core, _registration) = two_browsers("browser-remove-unreadable").await;
    let chrome = &core.shared.chrome_control;
    let inventory = chrome.dir().unwrap().join("browser-profiles.json");
    std::fs::remove_file(&inventory).unwrap();
    std::fs::create_dir(&inventory).unwrap();
    // As at a start with the folder already in place: the inventory loads as unreadable.
    *chrome.profiles.lock_or_recover() = profiles::Profiles::load(chrome.dir());
    assert_eq!(remove_chrome(&core).await, Some(not_saved()));
    assert!(
        inventory.is_dir(),
        "the unreadable inventory is left as it is"
    );
    std::fs::remove_dir(&inventory).unwrap();
    core.shutdown().await;
}
