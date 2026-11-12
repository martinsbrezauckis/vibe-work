//! Pairing permission through the real ask service and a controlled native presenter.
//! Exercises owner notifications, Debug API refusal and the private owner reply without a desktop.
use std::{sync::Arc, time::Duration};

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

const ID: &str = "11111111-1111-4111-8111-111111111111";
const PROFILE: &str = "22222222-2222-4222-8222-222222222222";

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

fn notify(core: &crate::Core, id: &str, browser: &str) {
    super::owner::observe(
        &Arc::downgrade(&core.shared),
        &json!({"method":"notifications/owner/pairingApproval", "params":{
            "requestId":id,"profileId":format!("{browser}:{PROFILE}")}}),
    )
    .unwrap();
}

async fn prompt(
    receive: &mut mpsc::UnboundedReceiver<(ConfirmationRequest, oneshot::Sender<Confirmation>)>,
) -> (ConfirmationRequest, oneshot::Sender<Confirmation>) {
    tokio::time::timeout(Duration::from_secs(2), receive.recv())
        .await
        .expect("pairing must reach the native presenter")
        .unwrap()
}

async fn settle() {
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
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::RemoveProfile {
            profile_id: profile.clone(),
        },
    )
    .await
    .unwrap();
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
    settle().await;
    assert!(calls.lock().unwrap().iter().any(|(method, params)| method == "owner.pairing.approve" && params["approved"] == true));
    core.shutdown().await;
}
