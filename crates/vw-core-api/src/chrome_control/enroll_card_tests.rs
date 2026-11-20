//! The browser card after Connect, pairing and Remove: when the pairing step shows, the 15 s connecting
//! fallback, whether Chrome still holds the extension, and the host repair Connect performs.
use super::super::addon::{AddonAction, AddonState};
use super::super::*;
use super::tests::{OURS, PROFILE, Rig, addon, ticket_answer};
use crate::Origin;
#[cfg(unix)]
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;

fn status(rig: &Rig, connected: bool) {
    let profiles = if connected {
        json!([{"profileId":PROFILE,"connected":true}])
    } else {
        json!([])
    };
    owner::update_pairing(
        &Arc::downgrade(&rig.core.shared),
        &json!({"structuredContent":{"profiles":profiles,"ownerFingerprint":OURS}}),
    );
}

/// Windows walk: the card said Pair in Chrome (with the code) while Chrome still asked to enable the extension. A
/// helper status answer always names our fingerprint; only the helper's waiting report is the pairing step.
#[tokio::test]
async fn the_pairing_step_shows_only_after_a_waiting_report() {
    let rig = Rig::new("chrome-addon-waiting", ticket_answer).await;
    status(&rig, false);
    let before = addon(&rig);
    assert_ne!(before.state, AddonState::NotPaired, "{before:?}");
    assert_ne!(
        rig.core.chrome_view().setup.pairing,
        "Waiting for you to pair in Chrome"
    );
    rig.waiting(false);
    let after = addon(&rig);
    assert_eq!(after.state, AddonState::NotPaired);
    assert_eq!(after.action, Some(AddonAction::OpenPairingPage));
    rig.core.shutdown().await;
}

/// Windows walk: after pairing, closing Chrome turned the card back to Pair in Chrome with the code.
#[tokio::test]
async fn a_paired_setup_with_chrome_closed_never_shows_the_pairing_step() {
    let rig = Rig::new("chrome-addon-closed", ticket_answer).await;
    rig.waiting(false);
    status(&rig, true);
    assert_eq!(addon(&rig).state, AddonState::Connected);
    // Chrome closed: its installed extension remains, but the helper has no connected profile.
    let chrome = &rig.core.shared.chrome_control;
    std::fs::create_dir_all(
        chrome
            .config
            .user_home
            .as_ref()
            .unwrap()
            .join("chrome-data/Default/Extensions")
            .join(&chrome.config.extension_id),
    )
    .unwrap();
    status(&rig, false);
    let closed = addon(&rig);
    assert_eq!(closed.state, AddonState::ChromeClosed);
    assert_eq!(closed.label, "Connected when Chrome is open");
    assert_eq!(closed.action, None);
    rig.core.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn labels_connecting_falls_back_to_pair_after_15_seconds() {
    let rig = Rig::new("chrome-addon-connecting", ticket_answer).await;
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    assert_eq!(addon(&rig).state, AddonState::Connecting);
    tokio::time::advance(std::time::Duration::from_millis(14_990)).await;
    assert_eq!(addon(&rig).state, AddonState::Connecting);
    tokio::time::advance(std::time::Duration::from_millis(20)).await;
    let fallback = addon(&rig);
    assert_eq!(
        fallback.state,
        AddonState::NotPaired,
        "Pair in Chrome + code"
    );
    assert!(rig.core.chrome_view().setup.fingerprint.is_some());
    rig.core.shutdown().await;
}

/// Windows walk: after Remove the app asked to remove the extension in Chrome although Chrome removed it itself.
#[tokio::test]
async fn remove_asks_to_remove_in_chrome_only_while_the_extension_is_still_there() {
    let rig = Rig::new("chrome-addon-remove", ticket_answer).await;
    let chrome = &rig.core.shared.chrome_control;
    let data = chrome
        .config
        .user_home
        .as_ref()
        .unwrap()
        .join("chrome-data");
    let extension = data
        .join("Profile 1/Extensions")
        .join(setup::EXTENSION_ID)
        .join("0.1.3_0");
    std::fs::create_dir_all(&extension).unwrap();
    rig.core
        .chrome_control(Origin::Ui, ChromeControlRequest::Remove {})
        .await
        .unwrap();
    assert_eq!(
        rig.core.chrome_view().setup.extension_left,
        Some(true),
        "a Store-added extension stays"
    );
    std::fs::remove_dir_all(data.join("Profile 1")).unwrap();
    assert_eq!(
        rig.core.chrome_view().setup.extension_left,
        None,
        "Chrome removed it"
    );
    rig.core.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn connect_restores_a_registration_after_the_browser_profile_was_reset() {
    let rig = Rig::new("connect-profile-reset", |_, _| json!({})).await;
    let chrome = &rig.core.shared.chrome_control;
    let dir = chrome.dir().unwrap();
    setup::install(&chrome.config, dir, &rig.core.shared.instructions.own_home).unwrap();
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(dir.join("setup.json")).unwrap()).unwrap();
    let path = receipt["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .map(std::path::PathBuf::from)
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n == "com.vibework.chrome_control.json")
        })
        .unwrap();
    let expected = std::fs::read(&path).unwrap();
    for damage in ["file", "folder", "contents", "start"] {
        if damage == "folder" {
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        } else if damage == "contents" {
            let mut changed: Value = serde_json::from_slice(&expected).unwrap();
            changed["allowed_origins"] = json!([]);
            std::fs::write(&path, changed.to_string()).unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        if damage == "start" {
            start_owner(&rig.core.shared).await;
        } else {
            rig.connect(Origin::Ui).await;
        }
        assert_eq!(std::fs::read(&path).unwrap(), expected, "{damage}");
    }
    #[cfg(feature = "your-edge")]
    {
        setup::register_edge(&chrome.config, dir).unwrap();
        let data = setup::user_data(&chrome.config, Some(dir), "Microsoft Edge").remove(0);
        let manifest = data
            .join("NativeMessagingHosts")
            .join(format!("{}.json", setup::HOST_NAME));
        std::fs::remove_dir_all(manifest.parent().unwrap()).unwrap();
        rig.core
            .chrome_control(Origin::Ui, ChromeControlRequest::OpenEdgeStore {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(manifest).unwrap(), expected);
    }
    rig.core.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn an_unrepairable_host_has_the_same_recovery_action_for_settings_and_the_agent() {
    let rig = Rig::new("host-repair-state", |_, _| json!({})).await;
    let chrome = &rig.core.shared.chrome_control;
    let state = chrome.dir().unwrap();
    setup::install(
        &chrome.config,
        state,
        &rig.core.shared.instructions.own_home,
    )
    .unwrap();
    let data = setup::user_data(&chrome.config, Some(state), "Google Chrome").remove(0);
    let extension = data
        .join("Default/Extensions")
        .join(&chrome.config.extension_id)
        .join("0.1.6");
    std::fs::create_dir_all(&extension).unwrap();
    std::fs::write(extension.join("manifest.json"), br#"{"version":"0.1.6"}"#).unwrap();
    let manifest = data
        .join("NativeMessagingHosts")
        .join(format!("{}.json", setup::HOST_NAME));
    std::fs::remove_file(&manifest).unwrap();
    let foreign = data.join("foreign.json");
    std::fs::write(&foreign, b"keep").unwrap();
    std::os::unix::fs::symlink(&foreign, &manifest).unwrap();
    let view = rig.core.chrome_view();
    let addon = view.setup.addon.unwrap();
    assert_eq!(addon.state, AddonState::HelperSetupRequired);
    assert_eq!(addon.action, Some(AddonAction::SetupAgain));
    assert_eq!(addon.label, "Helper setup required");
    assert!(
        addon
            .instruction
            .contains("connection setup is missing or changed")
    );
    assert!(addon.instruction.contains("Set up again"));
    assert!(addon.step.is_none());
    assert_eq!(commands::addon(&rig.core.shared), addon);
    assert!(
        rig.core
            .chrome_control(Origin::Ui, ChromeControlRequest::Connect {})
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&foreign).unwrap(), b"keep");
    std::fs::remove_file(manifest).unwrap();
    rig.connect(Origin::Ui).await;
    assert_ne!(
        commands::addon(&rig.core.shared).state,
        AddonState::HelperSetupRequired
    );
    rig.core.shutdown().await;
}
