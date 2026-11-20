//! A removed or declined profile pairs again only through a new native approval after the person's Connect, and the
//! native dialog names the asking profile when its folder is known.
//! Uses the real ask service and a controlled native presenter; no desktop or real browser.
use serde_json::json;
use tokio::sync::mpsc;
use vw_gate::Confirmation;

use super::pairing_approval_tests::{ID, PROFILE, Presenter, notify, prompt, settle};

/// Remove a profile, then the person's Connect: a removed profile asks only after it (`pairing_removed_tests`).
/// Setup may stop early here; only the removal must succeed.
async fn remove_then_connect(core: &crate::Core, profile: &str) {
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::RemoveProfile {
            profile_id: profile.into(),
        },
    )
    .await
    .unwrap();
    let _ = core
        .chrome_control(crate::Origin::Ui, super::ChromeControlRequest::Connect {})
        .await;
}

#[tokio::test]
async fn an_explicitly_removed_profile_can_pair_again_only_after_a_new_native_approval() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        "pairing-removed-profile",
        |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
        },
        Presenter(send),
    )
    .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    let profile = format!("chrome:{PROFILE}");
    super::test_support::observe_status(
        &core.shared,
        &json!({"profiles":[{"profileId":profile,"connected":true}]}),
    );
    let calls = core
        .shared
        .chrome_control
        .owner
        .answering(|tool, _| match tool {
            "owner.profileUnpair" => json!({"unpaired":true}),
            "owner.pairing.approve" => json!({"approved":true}),
            _ => json!({"applied":true}),
        });
    remove_then_connect(&core, &profile).await;
    notify(&core, ID, "chrome");
    let (_, answer) = prompt(&mut receive).await;
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "owner.pairing.approve")
    );
    answer.send(Confirmation::Approved).unwrap();
    settle().await;
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, args)| tool == "owner.pairing.approve" && args["approved"] == true)
    );
    assert!(
        super::profiles::view(&core.shared)
            .iter()
            .any(|row| row.profile_id == profile && row.enabled)
    );
    core.shutdown().await;
}

// PREDICT (red before the change): the native dialog said "Connect Chrome to Vibe Work?" for every profile, so with a
// second profile pairing the person could not tell which one asked. When the asking identity's own UUID is found in
// one profile folder's extension storage, the dialog names that folder's profile as the browser's picker shows it.
#[tokio::test]
async fn the_native_pairing_dialog_names_the_profile_that_asks_when_its_folder_is_known() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        "pairing-named",
        |config, dir| config.chrome_control.package = Some(dir.join("package")),
        Presenter(send),
    )
    .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    let chrome = &core.shared.chrome_control;
    let data = chrome
        .config
        .user_home
        .as_ref()
        .unwrap()
        .join("named-browser-data");
    let folder = data.join("Profile 2");
    let extension = &chrome.config.extension_id;
    let storage = folder
        .join("IndexedDB")
        .join(format!("chrome-extension_{extension}_0"));
    std::fs::create_dir_all(folder.join("Extensions").join(extension).join("0.1.7_0")).unwrap();
    std::fs::create_dir_all(&storage).unwrap();
    std::fs::write(
        storage.join("WCHYFF2I4HCHMUPPZAC67CSVYI6DH5O5Q6INA6VYGXVHAVFYHNPQ"),
        format!("SQLite format 3\0\u{ff}\u{14}\"${PROFILE}"),
    )
    .unwrap();
    std::fs::write(
        data.join("Local State"),
        json!({"profile":{"info_cache":{"Profile 2":{"name":"Work"}}}}).to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(chrome.dir().unwrap()).unwrap();
    std::fs::write(
        chrome.dir().unwrap().join("setup.json"),
        json!({"files":[],"browsers":["Google Chrome"],"userData":[data]}).to_string(),
    )
    .unwrap();
    notify(&core, ID, "chrome");
    let (request, _answer) = prompt(&mut receive).await;
    assert_eq!(
        request.summary,
        "Connect Chrome profile \u{201c}Work\u{201d} to Vibe Work?\n\nChrome needs your approval. Choose Connect to allow this connection."
    );
    core.shutdown().await;
}

// PREDICT (red before the change): after the person answers "Not now", the helper keeps that answer for its request's
// life, so the extension's reconnects never ask again and the profile stays unpaired until Vibe Work restarts. The card
// offered no way back. Now the card says the profile waits after Not now and offers Connect; Connect makes the helper
// forget the answer for that unpaired identity (`owner.profileUnpair`, no setup), returns guidance as a hint, and the
// next request from the helper opens the dialog again. Nothing asks again without that click.
#[tokio::test]
async fn after_not_now_the_persons_connect_lets_the_profile_ask_again_without_a_restart() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        "pairing-not-now",
        |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
        },
        Presenter(send),
    )
    .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    let profile = format!("chrome:{PROFILE}");
    super::test_support::observe_status(
        &core.shared,
        &json!({"profiles":[{"profileId":profile,"connected":true}]}),
    );
    let calls = core
        .shared
        .chrome_control
        .owner
        .answering(|tool, _| match tool {
            "owner.profileUnpair" => json!({"unpaired":true}),
            "owner.pairing.approve" => json!({"approved":false}),
            _ => json!({"applied":true}),
        });
    remove_then_connect(&core, &profile).await;
    notify(&core, ID, "chrome");
    let (_, answer) = prompt(&mut receive).await;
    answer.send(Confirmation::Declined).unwrap();
    settle().await;
    calls.lock().unwrap().clear();
    let card = || {
        let mut view = super::addon::view(super::AddonState::Connecting, "");
        super::profile_reason::reconnecting(
            &core.shared,
            &mut view,
            true,
            super::agent_browser::Browser::Chrome,
        );
        view
    };
    let card = card();
    assert_eq!(
        card.action,
        Some(super::addon::AddonAction::Connect),
        "{card:?}"
    );
    assert!(
        card.step.as_deref().unwrap_or_default().contains("Not now"),
        "{card:?}"
    );
    let result = core
        .chrome_control(crate::Origin::Ui, super::ChromeControlRequest::Connect {})
        .await
        .unwrap();
    let crate::CommandResult::ChromeControl { hint, .. } = result else {
        panic!("Connect answers with the browser view");
    };
    assert!(
        hint.as_deref()
            .unwrap_or_default()
            .contains("Check connection"),
        "{hint:?}"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![(
            "owner.profileUnpair".into(),
            json!({"profileId":profile,"keepPin":true})
        )],
        "Connect only lets the helper ask again"
    );
    let card = {
        let mut view = super::addon::view(super::AddonState::Connecting, "");
        super::profile_reason::reconnecting(
            &core.shared,
            &mut view,
            true,
            super::agent_browser::Browser::Chrome,
        );
        view
    };
    assert!(
        !card.step.as_deref().unwrap_or_default().contains("Not now"),
        "{card:?}"
    );
    let next = "33333333-3333-4333-8333-333333333333";
    notify(&core, next, "chrome");
    let (request, answer) = prompt(&mut receive).await;
    assert!(request.summary.starts_with("Connect Chrome"));
    answer.send(Confirmation::Declined).unwrap();
    settle().await;
    assert!(
        receive.try_recv().is_err(),
        "no dialog without a new request"
    );
    core.shutdown().await;
}
