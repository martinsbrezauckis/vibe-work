//! What the agent is told about a chat's browser profile: who chose it, and a profile that waits after "Not now"
//! while another profile of the same browser stays connected.
//!
//! PREDICT (written before the change):
//! - `a_pick_on_the_card_says_the_user_picked_it` and `a_default_bind_says_it_is_the_default_profile` fail: the
//!   open result carries only `profileId` and `profileName`, so `structuredContent.profileChoice` is null. With no
//!   word on who chose, an agent explained a person's pick as "the app's selection rule". A later `chrome_status`
//!   shows `selectedProfile.selectionState: "selected"` only.
//! - `not_now_while_another_profile_is_connected_reaches_the_agent` fails: in the Connected state the "Not now"
//!   card sets only the card's step line and Connect; the agent-facing instruction stays "Your Chrome is
//!   connected.", so the agent learns of the waiting profile only after the connected one goes offline.
#![allow(
    clippy::unwrap_used,
    reason = "test fixtures and held calls must exist for the test to mean anything"
)]
use super::test_support;
use crate::ask::tool_calls;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot};
use vw_ask::AnsweredBy;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

async fn core(name: &str) -> crate::Core {
    let helper = json!({"profiles":[
        {"profileId":"chrome:work","connected":true},
        {"profileId":"chrome:personal","connected":true}
    ]});
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
    let reported = helper.clone();
    test_support::answering(&core.shared, move |tool, args| {
        if tool == "chrome_status" {
            return json!({"structuredContent":reported});
        }
        if args["action"] == "open" {
            let task = format!("task-{}", args["profileId"].as_str().unwrap_or_default());
            return json!({"structuredContent":{"sessionId":task,"profileId":args["profileId"],"state":"active"}});
        }
        json!({"structuredContent":{}})
    });
    test_support::observe_status(&core.shared, &helper);
    {
        let mut saved = core.shared.chrome_control.profiles.lock().unwrap();
        for (id, name) in [("chrome:work", "Work"), ("chrome:personal", "Personal")] {
            saved.records.get_mut(id).unwrap().browser_name = Some(name.into());
        }
    }
    core
}

fn open(shared: &Arc<crate::core::Shared>) -> tokio::task::JoinHandle<Result<Value, String>> {
    let shared = shared.clone();
    tokio::spawn(async move {
        super::gateway::call(
            &shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"chrome"}),
        )
        .await
    })
}

/// The open profile card of chat `s`, once the held open has published it.
async fn card_id(shared: &Arc<crate::core::Shared>) -> String {
    for _ in 0..200 {
        let state = serde_json::to_value(crate::sessions::snapshot(shared, "s").unwrap()).unwrap();
        let id = state
            .pointer("/state/history")
            .and_then(Value::as_array)
            .and_then(|history| {
                history.iter().find(|entry| {
                    entry["detail"]["effect"]["kind"] == "browserProfile"
                        && entry["state"]["status"] == "open"
                })
            })
            .and_then(|entry| entry["callbackId"].as_str().map(str::to_owned));
        if let Some(id) = id {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no browser profile card opened");
}

#[tokio::test]
async fn a_pick_on_the_card_says_the_user_picked_it() {
    let core = core("agent-text-picked").await;
    let held = open(&core.shared);
    let id = card_id(&core.shared).await;
    tool_calls::answer_by(
        &core.shared,
        crate::Origin::Ui,
        "s",
        &id,
        &json!({"type":"approval","decision":{"type":"approve"},"profileId":"chrome:personal"}),
        AnsweredBy::Person,
    )
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let body = &result["vwMcpResult"]["structuredContent"];
    assert_eq!(
        body["profileChoice"], "The user picked Personal for this chat on the card.",
        "{result}"
    );
    // The model reads the text block; it carries the same sentence.
    assert!(
        result["vwMcpResult"]
            .to_string()
            .contains("The user picked Personal for this chat on the card."),
        "{result}"
    );
    // A later status read keeps the same fact on the chat's selected profile.
    let status = super::gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        status["vwMcpResult"]["structuredContent"]["selectedProfile"]["profileChoice"],
        "The user picked Personal for this chat on the card.",
        "{status}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_default_bind_says_it_is_the_default_profile() {
    let core = core("agent-text-default").await;
    core.shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .defaults
        .insert("chrome".into(), "chrome:work".into());
    let result = tokio::time::timeout(Duration::from_secs(10), open(&core.shared))
        .await
        .expect("the default decides without a card")
        .unwrap()
        .unwrap();
    assert_eq!(
        result["vwMcpResult"]["structuredContent"]["profileChoice"],
        "This chat uses Work, the user's default profile.",
        "{result}"
    );
    core.shutdown().await;
}

const ID: &str = "11111111-1111-4111-8111-111111111111";
const ASKING: &str = "chrome:22222222-2222-4222-8222-222222222222";

struct Presenter(mpsc::UnboundedSender<oneshot::Sender<Confirmation>>);
impl Confirmer for Presenter {
    fn confirm(
        &self,
        _request: &ConfirmationRequest,
    ) -> impl std::future::Future<Output = Confirmation> + Send {
        let (send, receive) = oneshot::channel();
        self.0.send(send).unwrap();
        async move { receive.await.unwrap_or(Confirmation::Declined) }
    }
}

#[tokio::test]
async fn not_now_while_another_profile_is_connected_reaches_the_agent() {
    let (send, mut receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        "agent-text-not-now",
        |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
        },
        Presenter(send),
    )
    .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    test_support::observe_status(
        &core.shared,
        &json!({"profiles":[{"profileId":"chrome:work","connected":true}]}),
    );
    let _calls = core
        .shared
        .chrome_control
        .owner
        .answering(|tool, _| match tool {
            "owner.pairing.approve" => json!({"approved":false}),
            _ => json!({"applied":true}),
        });
    super::owner::observe(
        &Arc::downgrade(&core.shared),
        &json!({"method":"notifications/owner/pairingApproval",
            "params":{"requestId":ID,"profileId":ASKING}}),
    )
    .unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(2), receive.recv())
        .await
        .expect("pairing must reach the native presenter")
        .unwrap();
    answer.send(Confirmation::Declined).unwrap();
    // The helper's reply to "Not now" is recorded by a spawned task: wait for the card to show it (a fixed
    // pause is too short when the whole test binary runs in parallel).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let view = loop {
        let mut view = super::addon::view(super::AddonState::Connected, "");
        super::profile_reason::window_card(
            &core.shared.chrome_control,
            &mut view,
            super::agent_browser::Browser::Chrome,
        );
        if view.step.is_some() || tokio::time::Instant::now() >= deadline {
            break view;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        view.instruction,
        "Your Chrome is connected. A Chrome profile asked to connect to Vibe Work and the user chose Not now. Ask the user to click Connect Chrome in Settings > Browser to be asked again.",
        "{view:?}"
    );
    // The card for the person keeps its own label and step.
    assert_eq!(view.label, "Connected", "{view:?}");
    assert!(
        view.step.as_deref().unwrap_or_default().contains("Not now"),
        "{view:?}"
    );
    core.shutdown().await;
}
