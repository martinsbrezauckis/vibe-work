//! Pairing permission through the real ask service and a controlled native presenter.
//! Exercises owner notifications, Debug API refusal and the private owner reply without a desktop.
use std::{sync::Arc, time::Duration};

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

pub(super) const ID: &str = "11111111-1111-4111-8111-111111111111";
pub(super) const PROFILE: &str = "22222222-2222-4222-8222-222222222222";

pub(super) struct Presenter(
    pub(super) mpsc::UnboundedSender<(ConfirmationRequest, oneshot::Sender<Confirmation>)>,
);
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

pub(super) fn notify(core: &crate::Core, id: &str, browser: &str) {
    super::owner::observe(
        &Arc::downgrade(&core.shared),
        &json!({"method":"notifications/owner/pairingApproval", "params":{
            "requestId":id,"profileId":format!("{browser}:{PROFILE}")}}),
    )
    .unwrap();
}

pub(super) async fn prompt(
    receive: &mut mpsc::UnboundedReceiver<(ConfirmationRequest, oneshot::Sender<Confirmation>)>,
) -> (ConfirmationRequest, oneshot::Sender<Confirmation>) {
    tokio::time::timeout(Duration::from_secs(2), receive.recv())
        .await
        .expect("pairing must reach the native presenter")
        .unwrap()
}

pub(super) async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn legacy_pairing_requires_one_native_answer_and_debug_api_cannot_supply_it() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core =
        crate::offline_core::start_with_confirmer("pairing-native", |_, _| {}, Presenter(send))
            .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    let calls = core
        .shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"approved":true}));
    notify(&core, ID, "chrome");
    let (request, answer) = prompt(&mut receive).await;
    assert_eq!(request.method, "chrome/control/pair");
    assert_eq!(
        request.summary,
        "Connect Chrome to Vibe Work?\n\nChrome needs your approval. Choose Connect to allow this connection."
    );
    assert!(request.always.is_none());
    assert!(!request.summary.contains(PROFILE));
    let asks = core.asks();
    assert_eq!(asks.len(), 1);
    assert!(asks[0].person_only);
    assert!(
        core.answer_ask(
            &asks[0].id,
            vw_ask::Answer {
                answer: vw_ask::Choice::Once,
                note: None
            }
        )
        .await
        .is_err()
    );
    notify(&core, ID, "chrome");
    settle().await;
    assert!(receive.try_recv().is_err());
    assert!(
        calls.lock().unwrap().is_empty(),
        "no approval before the native answer"
    );
    answer.send(Confirmation::Approved).unwrap();
    settle().await;
    assert_eq!(
        *calls.lock().unwrap(),
        vec![(
            "owner.pairing.approve".into(),
            json!({"requestId":ID,"approved":true})
        )]
    );
    notify(&core, ID, "chrome");
    settle().await;
    assert!(receive.try_recv().is_err());
    core.shared.chrome_control.owner.stop().await;
}

#[tokio::test]
async fn declining_and_helper_loss_never_authorize_pairing() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core =
        crate::offline_core::start_with_confirmer("pairing-decline", |_, _| {}, Presenter(send))
            .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    let calls = core
        .shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"approved":false}));
    notify(&core, ID, "chrome");
    let (_, answer) = prompt(&mut receive).await;
    answer.send(Confirmation::Declined).unwrap();
    settle().await;
    assert_eq!(
        *calls.lock().unwrap(),
        vec![(
            "owner.pairing.approve".into(),
            json!({"requestId":ID,"approved":false})
        )]
    );
    let next = "33333333-3333-4333-8333-333333333333";
    notify(&core, next, "chrome");
    let (_, answer) = prompt(&mut receive).await;
    core.shared.chrome_control.owner.stop().await;
    settle().await;
    assert!(
        answer.send(Confirmation::Approved).is_err(),
        "helper loss closes the old dialog"
    );
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert_eq!(core.asks(), [] as [vw_ask::Ask; 0]);
}

#[tokio::test]
async fn unsupported_and_malformed_pairing_requests_do_not_prompt() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core =
        crate::offline_core::start_with_confirmer("pairing-invalid", |_, _| {}, Presenter(send))
            .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    let calls = core
        .shared
        .chrome_control
        .owner
        .answering(|_, _| Value::Null);
    for browser in ["safari", "unknown", "Chrome"] {
        notify(&core, ID, browser);
    }
    if !cfg!(feature = "your-firefox") {
        notify(&core, ID, "firefox");
    }
    notify(&core, "invalid", "chrome");
    settle().await;
    assert!(receive.try_recv().is_err());
    assert!(calls.lock().unwrap().is_empty());
    core.shared.chrome_control.owner.stop().await;
}

#[tokio::test]
async fn a_ticket_finishing_the_same_pairing_closes_the_unused_dialog() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core =
        crate::offline_core::start_with_confirmer("pairing-ticket", |_, _| {}, Presenter(send))
            .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    let calls = core.shared.chrome_control.owner.answering(|_, _| json!({}));
    notify(&core, ID, "chrome");
    let (_, answer) = prompt(&mut receive).await;
    super::owner::observe(&Arc::downgrade(&core.shared), &json!({
        "method":"notifications/owner/pairing", "params":{
            "profileId":format!("chrome:{PROFILE}"),"state":"paired","fingerprint":"ab".repeat(32)
        }
    })).unwrap();
    settle().await;
    assert!(answer.send(Confirmation::Approved).is_err());
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .all(|(method, _)| method == "chrome_status"),
        "finishing pairing may refresh metadata but must never approve the unused dialog"
    );
    notify(&core, ID, "chrome");
    settle().await;
    assert!(receive.try_recv().is_err());
    core.shared.chrome_control.owner.stop().await;
}

#[tokio::test]
async fn an_approval_that_cannot_be_saved_shows_a_recovery_notice() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        "pairing-save-failed",
        |_, _| {},
        Presenter(send),
    )
    .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    core.shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"rpcError":{"message":"Pairing could not be saved"}}));
    let mut events = core.shared.events.subscribe();
    notify(&core, ID, "chrome");
    let (_, answer) = prompt(&mut receive).await;
    answer.send(Confirmation::Approved).unwrap();
    settle().await;
    let mut notices = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let crate::CoreEvent::Alert { message, .. } = event {
            notices.push(message);
        }
    }
    assert_eq!(
        notices,
        ["Chrome could not connect. Restart Vibe Work and try again."]
    );
    core.shared.chrome_control.owner.stop().await;
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn edge_has_its_own_confirmation_and_disabling_chrome_cancels_only_chrome() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core =
        crate::offline_core::start_with_confirmer("pairing-browsers", |_, _| {}, Presenter(send))
            .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    core.shared
        .chrome_control
        .set_browser_enabled(super::agent_browser::Browser::Edge, true)
        .unwrap();
    let calls = core
        .shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"approved":true}));
    notify(&core, ID, "chrome");
    let (_, chrome_answer) = prompt(&mut receive).await;
    let edge_id = "33333333-3333-4333-8333-333333333333";
    notify(&core, edge_id, "edge");
    let (edge, edge_answer) = prompt(&mut receive).await;
    assert!(edge.summary.starts_with("Connect Edge to Vibe Work?"));
    assert!(!edge.summary.contains("Chrome"));
    core.shared.chrome_control.set_enabled(false).unwrap();
    settle().await;
    assert!(chrome_answer.send(Confirmation::Approved).is_err());
    edge_answer.send(Confirmation::Approved).unwrap();
    settle().await;
    assert_eq!(
        *calls.lock().unwrap(),
        vec![(
            "owner.pairing.approve".into(),
            json!({"requestId":edge_id,"approved":true})
        )]
    );
    core.shared.chrome_control.owner.stop().await;
}

#[cfg(feature = "your-firefox")]
#[tokio::test]
async fn firefox_pairing_uses_the_same_person_only_confirmation() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        "firefox-pairing-native",
        |_, _| {},
        Presenter(send),
    )
    .await;
    let calls = core
        .shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"approved":true}));
    notify(&core, ID, "firefox");
    let (request, answer) = prompt(&mut receive).await;
    assert_eq!(request.method, "chrome/control/pair");
    assert!(request.summary.starts_with("Connect Firefox to Vibe Work?"));
    assert!(core.asks()[0].person_only);
    answer.send(Confirmation::Approved).unwrap();
    settle().await;
    assert_eq!(
        calls.lock().unwrap()[0],
        (
            "owner.pairing.approve".into(),
            json!({"requestId":ID,"approved":true})
        )
    );
    core.shared.chrome_control.owner.stop().await;
}

#[cfg(feature = "your-safari")]
#[tokio::test]
async fn safari_repair_approval_uses_the_same_person_only_card() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        "safari-native-pairing",
        |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
        },
        Presenter(send),
    )
    .await;
    let dir = core.shared.chrome_control.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
    let calls = core
        .shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"approved":true}));
    notify(&core, ID, "safari");
    let (request, answer) = prompt(&mut receive).await;
    assert_eq!(request.method, super::BROWSER_PAIRING_DIALOG_METHOD);
    assert!(request.summary.contains("Safari needs your approval"));
    assert!(core.asks().iter().all(|ask| ask.person_only));
    assert!(calls.lock().unwrap().is_empty());
    answer.send(Confirmation::Approved).unwrap();
    // PREDICT: with the fixed 50 ms `settle` this failed in every full parallel lib run on Windows (the approval task
    // had not finished); it waits for the approval's own end now.
    super::test_support::until_safari_approved(&core.shared).await;
    assert!(calls.lock().unwrap().iter().any(|(method, params)| method == "owner.pairing.approve" && params["approved"] == true));
    core.shutdown().await;
}

// PREDICT (red before the change): after the person chose Connect on "Connect Safari to Vibe Work?", Safari may not
// run the extension's page again until its toolbar button is clicked, so Settings kept the unpaired "Pair in Safari"
// card and a person thought pairing had failed. Now the card says to click Your Safari in the toolbar to finish, as a
// help step, until the extension connects; once connected the step is gone and does not come back.
#[cfg(feature = "your-safari")]
#[tokio::test]
async fn an_approved_safari_pairing_says_to_finish_in_the_toolbar_until_safari_connects() {
    use super::agent_browser::Browser;
    let (send, mut receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        "safari-finish-step",
        |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
        },
        Presenter(send),
    )
    .await;
    let dir = core.shared.chrome_control.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
    let _calls = core
        .shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"approved":true}));
    let unpaired = || super::addon::view_for(super::AddonState::NotPaired, "", Browser::Safari);
    let card = || {
        let mut view = unpaired();
        super::safari::finishing(&core.shared, &mut view, true);
        view
    };
    assert_eq!(card(), unpaired(), "nothing approved yet");
    notify(&core, ID, "safari");
    let (_, answer) = prompt(&mut receive).await;
    answer.send(Confirmation::Approved).unwrap();
    // PREDICT: with the fixed 50 ms `settle` this failed in every full parallel lib run on Windows (the approval task
    // had not finished); it waits for the approval's own end now.
    super::test_support::until_safari_approved(&core.shared).await;
    let view = card();
    assert_eq!(
        view.step.as_deref(),
        Some("Click Your Safari in Safari's toolbar to finish.")
    );
    assert_ne!(view.label, unpaired().label, "it no longer looks unpaired");
    let mut connected = super::addon::view_for(super::AddonState::Connected, "", Browser::Safari);
    super::safari::finishing(&core.shared, &mut connected, true);
    assert_eq!(connected.step, None);
    assert_eq!(card(), unpaired(), "the step is gone once Safari connected");
    core.shutdown().await;
}
