//! One admission pass over a reply's invalidators (`PendingCall::finish`): when several fired, the reason the
//! agent reads keeps the fixed order — profile binding, profile switch, browser switch, the person's Stop — and a
//! Stop of the whole chat or turn reads "work ended". Guards the single-pass rewrite of `finish` (A2-02).
use super::{agent_browser::Browser, owner, person_stop, test_support};
use serde_json::json;

async fn chat() -> crate::Core {
    let core = crate::offline_core::start_with("admission-order", |config, root| {
        config.chrome_control.package = Some(root.join("package"));
    })
    .await;
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    let _calls = test_support::answering(&core.shared, |_, _| json!({"applied":true}));
    core
}

fn person_stop(core: &crate::Core) {
    let _ = core
        .shared
        .chrome_control
        .person_stopped(&[person_stop::Stopped {
            chat: "s".into(),
            browser: Browser::Chrome,
            turn: None,
            connection_ended: false,
        }]);
}

fn capture(core: &crate::Core) -> person_stop::PendingCall {
    person_stop::PendingCall::new(
        &core.shared,
        "s",
        "chrome_execute",
        &json!({"sessionId":"t","action":"page.read"}),
    )
}

fn reply() -> owner::Answer {
    owner::Answer::Helper(json!({"structuredContent":{"status":"completed"}}))
}

#[tokio::test]
async fn the_browser_switch_reason_precedes_the_persons_stop() {
    let core = chat().await;
    let pending = capture(&core);
    person_stop(&core);
    core.browser_enabled(Browser::Chrome, false).await.unwrap();
    let reason = pending
        .finish(&core.shared, "s", Browser::Chrome, &reply())
        .unwrap_err();
    assert!(reason.contains("stopped by its switch"), "{reason}");
    core.shutdown().await;
}

#[tokio::test]
async fn the_persons_stop_alone_gives_its_own_reason() {
    let core = chat().await;
    let pending = capture(&core);
    person_stop(&core);
    let reason = pending
        .finish(&core.shared, "s", Browser::Chrome, &reply())
        .unwrap_err();
    assert_eq!(reason, person_stop::refusal(Browser::Chrome));
    core.shutdown().await;
}

#[tokio::test]
async fn a_chat_stop_without_a_reason_reads_work_ended() {
    let core = chat().await;
    let pending = capture(&core);
    crate::stops::stopped(&core.shared, "s");
    let reason = pending
        .finish(&core.shared, "s", Browser::Chrome, &reply())
        .unwrap_err();
    assert_eq!(reason, person_stop::ended(Browser::Chrome));
    core.shutdown().await;
}

#[tokio::test]
async fn nothing_changed_admits_the_reply() {
    let core = chat().await;
    let pending = capture(&core);
    pending
        .finish(&core.shared, "s", Browser::Chrome, &reply())
        .unwrap();
    core.shutdown().await;
}
