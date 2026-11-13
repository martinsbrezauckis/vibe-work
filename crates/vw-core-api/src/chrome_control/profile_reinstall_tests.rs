//! A Chrome profile whose Vibe Work extension was removed and installed again pairs under a new identity in the same
//! profile folder. Settings, the card and agent text show that one profile under the name Chrome shows for it from the
//! moment the person approves the pairing, and removing it leaves no row of that folder behind.
//!
//! PREDICT (written before the change): the first three tests fail on the base, the last one is a guard.
//! - The pairing dialog finds the asking identity's folder and names it, but the approval keeps nothing of that, so
//!   the new identity has no folder until a later owner status runs the folder search. Until then it is named by its
//!   pairing number ("Chrome profile 3"), and the old identity of that folder still shows as an Offline row.
//! - When the folder search does prove the new identity in the same folder, `profile_names::refresh` does not read
//!   `Local State` again: it remembers which folders it read, and the old identity's read already covered this folder.
//!   The new identity keeps the folder's first pairing number ("Chrome profile 2") until Chrome rewrites `Local State`.
//! - Removing the new identity makes the old one shown again: an older identity is retired only while the newer one
//!   in its folder is not removed, so the old record returns as "Person 1 · Offline · No window of this profile open".
//! - With every other profile removed too, the browser card does not see "every pairing removed": the retired old
//!   identity is neither removed nor shown, and `profiles::all_removed` counted it as a pairing still kept.
//! - A different real profile keeps its own name and state throughout.
//! - Remove of the new identity also unpairs the older, hidden identity of the same folder in the helper; Remove of
//!   an ordinary profile unpairs only that profile.
//!
//! Uses temporary browser folders laid out like Chrome's and a controlled native presenter, no native browser.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "temporary browser folders, fixtures and held prompts must exist for the test to mean anything"
)]
use super::profile_names_tests::browser_data;
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

const HOME_UUID: &str = "6fa459ea-ee8a-4ca4-894e-db77e160355e";
const OLD_UUID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
const NEW_UUID: &str = "9c3d2e1f-0a4b-4c5d-8e6f-7a8b9c0d1e2f";
const HOME: &str = "chrome:6fa459ea-ee8a-4ca4-894e-db77e160355e";
const OLD: &str = "chrome:1b4e28ba-2fa1-41d2-883f-0016d3cca427";
const NEW: &str = "chrome:9c3d2e1f-0a4b-4c5d-8e6f-7a8b9c0d1e2f";
const REQUEST: &str = "33333333-3333-4333-8333-333333333333";

type Prompts = mpsc::UnboundedReceiver<(ConfirmationRequest, oneshot::Sender<Confirmation>)>;
struct Presenter(mpsc::UnboundedSender<(ConfirmationRequest, oneshot::Sender<Confirmation>)>);
impl Confirmer for Presenter {
    fn confirm(
        &self,
        request: &ConfirmationRequest,
    ) -> impl std::future::Future<Output = Confirmation> + Send {
        let (send, receive) = oneshot::channel();
        self.0.send((request.clone(), send)).unwrap();
        async move { receive.await.unwrap_or(Confirmation::Declined) }
    }
}

/// Two real Chrome profiles, both paired and proven: "Mein Chrome" (Default) and "Person 1" (Profile 2).
async fn paired_core(name: &str) -> (crate::Core, Prompts, std::path::PathBuf) {
    let (send, receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        name,
        |config, dir| config.chrome_control.package = Some(dir.join("package")),
        Presenter(send),
    )
    .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    core.shared
        .chrome_control
        .owner
        .answering(|name, _| match name {
            "owner.browserAvailability" => json!({"applied":true}),
            "owner.profileUnpair" => json!({"unpaired":true}),
            _ => json!({"approved":true}),
        });
    super::test_support::chat(&core.shared, "s");
    let data = browser_data(
        &core,
        &[("Default", HOME_UUID), ("Profile 2", OLD_UUID)],
        &json!({"Default":{"name":"Mein Chrome"},"Profile 2":{"name":"Person 1"}}),
    );
    status(&core, &[(HOME, true), (OLD, true)]);
    assert_eq!(
        rows(&core),
        [
            (HOME.into(), "Mein Chrome".into()),
            (OLD.into(), "Person 1".into())
        ]
    );
    (core, receive, data)
}

fn status(core: &crate::Core, rows: &[(&str, bool)]) {
    let rows: Vec<Value> = rows
        .iter()
        .map(|(id, connected)| json!({"profileId":id,"connected":connected}))
        .collect();
    super::test_support::observe_status(&core.shared, &json!({ "profiles": rows }));
}

fn rows(core: &crate::Core) -> Vec<(String, String)> {
    super::profiles::view(&core.shared)
        .into_iter()
        .filter(|row| row.browser == "chrome")
        .map(|row| (row.profile_id, row.name))
        .collect()
}

/// The extension was removed from "Person 1" and installed again: its storage now holds a new identity's UUID.
/// Chrome's `Local State` is not touched by that.
fn reinstall(core: &crate::Core, data: &Path) {
    let id = core.shared.chrome_control.config.extension_id.clone();
    std::fs::write(
        data.join("Profile 2/IndexedDB")
            .join(format!("chrome-extension_{id}_0"))
            .join("WCHYFF2I4HCHMUPPZAC67CSVYI6DH5O5Q6INA6VYGXVHAVFYHNPQ"),
        format!("SQLite format 3\0\u{ff}\u{14}\"${NEW_UUID}"),
    )
    .unwrap();
}

/// The new identity asks to pair; the person approves the native dialog. Returns the dialog's text.
async fn approve(core: &crate::Core, prompts: &mut Prompts) -> String {
    super::owner::observe(
        &Arc::downgrade(&core.shared),
        &json!({"method":"notifications/owner/pairingApproval",
            "params":{"requestId":REQUEST,"profileId":NEW}}),
    )
    .unwrap();
    let (request, answer) = tokio::time::timeout(Duration::from_secs(5), prompts.recv())
        .await
        .expect("pairing must reach the native presenter")
        .unwrap();
    answer.send(Confirmation::Approved).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while core
        .shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .records
        .get(NEW)
        .is_none_or(|record| record.paired_at.is_none())
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the approved pairing must be recorded"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    request.summary
}

#[tokio::test]
async fn a_profile_paired_again_after_a_reinstall_has_its_chrome_name_from_the_approval() {
    let (core, mut prompts, data) = paired_core("reinstall-approval-name").await;
    reinstall(&core, &data);
    status(&core, &[(HOME, true)]);
    let summary = approve(&core, &mut prompts).await;
    assert!(
        summary.starts_with("Connect Chrome profile \u{201c}Person 1\u{201d}"),
        "{summary}"
    );
    // No owner status has run since the approval: the folder the dialog found names the profile at once.
    assert_eq!(
        rows(&core),
        [
            (HOME.into(), "Mein Chrome".into()),
            (NEW.into(), "Person 1".into())
        ]
    );
    assert_eq!(
        super::profiles::label(&core.shared.chrome_control, NEW),
        "Chrome · Person 1"
    );
    // The new identity connects; nothing changes the name or brings the old row back.
    status(&core, &[(HOME, true), (NEW, true), (OLD, false)]);
    assert_eq!(
        rows(&core),
        [
            (HOME.into(), "Mein Chrome".into()),
            (NEW.into(), "Person 1".into())
        ]
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_folder_proven_again_for_a_new_identity_reads_its_chrome_name() {
    let (core, _prompts, data) = paired_core("reinstall-search-name").await;
    reinstall(&core, &data);
    // Paired in a way that left no folder (the dialog could not read the files in time): the folder search proves it.
    core.shared.chrome_control.profiles.lock().unwrap().scanned = None;
    status(&core, &[(HOME, true), (NEW, true), (OLD, false)]);
    let folder = core.shared.chrome_control.profiles.lock().unwrap().records[NEW]
        .folder
        .clone();
    assert_eq!(folder, Some(data.join("Profile 2")));
    assert_eq!(
        rows(&core),
        [
            (HOME.into(), "Mein Chrome".into()),
            (NEW.into(), "Person 1".into())
        ]
    );
    core.shutdown().await;
}

#[tokio::test]
async fn removing_a_profile_paired_again_after_a_reinstall_leaves_no_row_of_its_folder() {
    let (core, mut prompts, data) = paired_core("reinstall-remove").await;
    let unpaired = record_unpairs(&core);
    reinstall(&core, &data);
    status(&core, &[(HOME, true)]);
    approve(&core, &mut prompts).await;
    status(&core, &[(HOME, true), (NEW, true), (OLD, false)]);
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::RemoveProfile {
            profile_id: NEW.into(),
        },
    )
    .await
    .unwrap();
    // The older identity is hidden but was still approved in the helper: Remove unpairs it too, so no approved
    // pairing of Person 1 is left without a row the person could remove.
    assert_eq!(
        *unpaired.lock().unwrap(),
        [NEW.to_string(), OLD.to_string()]
    );
    // A slow helper may still list both as offline after the removal; the window of Person 1 stays open.
    status(&core, &[(HOME, true), (NEW, false), (OLD, false)]);
    assert_eq!(rows(&core), [(HOME.into(), "Mein Chrome".into())]);
    assert!(super::profiles::retired(&core.shared.chrome_control, OLD));
    let view = super::profiles::view(&core.shared);
    assert_eq!(view.len(), 1, "{view:?}");
    assert_eq!(view[0].state, super::ProfileState::Connected);
    core.shutdown().await;
}

#[tokio::test]
async fn removing_every_shown_profile_after_a_reinstall_counts_as_every_pairing_removed() {
    let (core, mut prompts, data) = paired_core("reinstall-remove-all").await;
    remove(&core, HOME).await;
    reinstall(&core, &data);
    status(&core, &[(HOME, false)]);
    approve(&core, &mut prompts).await;
    status(&core, &[(HOME, false), (NEW, true), (OLD, false)]);
    assert!(!super::profiles::all_removed(
        &core.shared.chrome_control,
        super::agent_browser::Browser::Chrome
    ));
    remove(&core, NEW).await;
    status(&core, &[(HOME, false), (NEW, false), (OLD, false)]);
    assert_eq!(rows(&core), Vec::<(String, String)>::new());
    assert!(super::profiles::all_removed(
        &core.shared.chrome_control,
        super::agent_browser::Browser::Chrome
    ));
    core.shutdown().await;
}

/// Records the profile of every `owner.profileUnpair` the core sends; answers like `paired_core`.
fn record_unpairs(core: &crate::Core) -> Arc<std::sync::Mutex<Vec<String>>> {
    let unpaired = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&unpaired);
    core.shared
        .chrome_control
        .owner
        .answering(move |name, params| match name {
            "owner.browserAvailability" => json!({"applied":true}),
            "owner.profileUnpair" => {
                seen.lock()
                    .unwrap()
                    .push(params["profileId"].as_str().unwrap_or_default().to_string());
                json!({"unpaired":true})
            }
            _ => json!({"approved":true}),
        });
    unpaired
}

async fn remove(core: &crate::Core, id: &str) {
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::RemoveProfile {
            profile_id: id.into(),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn removing_a_profile_that_was_not_reinstalled_keeps_the_other_profile_as_it_was() {
    let (core, _prompts, _data) = paired_core("reinstall-remove-other").await;
    let unpaired = record_unpairs(&core);
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::RemoveProfile {
            profile_id: OLD.into(),
        },
    )
    .await
    .unwrap();
    status(&core, &[(HOME, true), (OLD, false)]);
    assert_eq!(rows(&core), [(HOME.into(), "Mein Chrome".into())]);
    // A different real profile is never retired by another folder's pairings, nor unpaired with it.
    assert!(!super::profiles::retired(&core.shared.chrome_control, HOME));
    assert_eq!(*unpaired.lock().unwrap(), [OLD.to_string()]);
    core.shutdown().await;
}
