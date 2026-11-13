//! A browser profile the person removes in Settings does not ask to connect again by itself.
//! Uses the offline core, a scripted private owner and a controlled native presenter; no desktop or real browser.
//!
//! PREDICT (red before the change): the extension of a removed profile that is still installed reconnects about a
//! second after Remove, the helper raises a new pairing request, and Vibe Work opened "Connect Chrome profile … to
//! Vibe Work?" although the person just removed it and pressed nothing. Now Remove (row or whole browser card) marks
//! the identity for this app run: its requests are answered "Not now" without a dialog, the card says the removed
//! profile is still connected in Chrome and offers Connect, and only that Connect lets the next request ask. A profile
//! that was never removed still asks at once.
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

use super::agent_browser::Browser;

const PROFILE: &str = "22222222-2222-4222-8222-222222222222";
const OTHER: &str = "44444444-4444-4444-8444-444444444444";
const REQUESTS: [&str; 3] = [
    "11111111-1111-4111-8111-111111111111",
    "33333333-3333-4333-8333-333333333333",
    "55555555-5555-4555-8555-555555555555",
];

type Prompts = mpsc::UnboundedReceiver<(ConfirmationRequest, oneshot::Sender<Confirmation>)>;
type Calls = Arc<Mutex<Vec<(String, Value)>>>;

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

async fn start(name: &str) -> (crate::Core, Prompts) {
    let (send, receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        name,
        |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
        },
        Presenter(send),
    )
    .await;
    core.shared.chrome_control.set_enabled(true).unwrap();
    (core, receive)
}

fn answering(core: &crate::Core) -> Calls {
    core.shared
        .chrome_control
        .owner
        .answering(|tool, _| match tool {
            "owner.profileUnpair" | "owner.browserUnpair" => json!({"unpaired":true}),
            "owner.pairing.approve" => json!({"approved":false}),
            _ => json!({"applied":true}),
        })
}

fn connected(core: &crate::Core, profiles: &[&str]) {
    let rows: Vec<Value> = profiles
        .iter()
        .map(|profile| json!({"profileId":profile,"connected":true}))
        .collect();
    super::test_support::observe_status(&core.shared, &json!({ "profiles": rows }));
}

fn notify(core: &crate::Core, id: &str, profile: &str) {
    super::owner::observe(
        &Arc::downgrade(&core.shared),
        &json!({"method":"notifications/owner/pairingApproval", "params":{
            "requestId":id,"profileId":profile}}),
    )
    .unwrap();
}

async fn no_prompt(receive: &mut Prompts, why: &str) {
    let seen = tokio::time::timeout(Duration::from_millis(300), receive.recv()).await;
    assert!(
        seen.is_err(),
        "{why}: {:?}",
        seen.ok().flatten().map(|(r, _)| r.summary)
    );
}

async fn prompt(receive: &mut Prompts) -> (ConfirmationRequest, oneshot::Sender<Confirmation>) {
    tokio::time::timeout(Duration::from_secs(2), receive.recv())
        .await
        .expect("the request must reach the native presenter")
        .unwrap()
}

fn card(core: &crate::Core, browser: Browser) -> super::AddonView {
    let mut view = super::addon::view(super::AddonState::Connecting, "");
    super::profile_reason::reconnecting(&core.shared, &mut view, true, browser);
    view
}

fn declined_quietly(calls: &Calls, id: &str) -> bool {
    calls.lock().unwrap().iter().any(|(tool, args)| {
        tool == "owner.pairing.approve" && *args == json!({"requestId":id,"approved":false})
    })
}

async fn remove_row(core: &crate::Core, profile: &str) {
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::RemoveProfile {
            profile_id: profile.into(),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_removed_profile_does_not_ask_by_itself_and_connect_lets_it_ask_again() {
    let (core, mut receive) = start("rmquiet-row").await;
    let profile = format!("chrome:{PROFILE}");
    connected(&core, &[&profile]);
    let calls = answering(&core);
    remove_row(&core, &profile).await;
    calls.lock().unwrap().clear();
    notify(&core, REQUESTS[0], &profile);
    no_prompt(
        &mut receive,
        "the extension's reconnect after Remove must not ask",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        declined_quietly(&calls, REQUESTS[0]),
        "the helper hears Not now so it stops asking: {:?}",
        calls.lock().unwrap()
    );
    let view = card(&core, Browser::Chrome);
    assert_eq!(view.label, "Waiting for your approval", "{view:?}");
    assert_eq!(
        view.action,
        Some(super::addon::AddonAction::Connect),
        "{view:?}"
    );
    assert_eq!(
        view.step.as_deref(),
        Some(
            "You removed a Chrome profile that is still connected in Chrome. Click Connect Chrome to pair it again."
        ),
        "{view:?}"
    );
    // A later automatic request (helper restart, request life over) stays quiet too.
    notify(&core, REQUESTS[1], &profile);
    no_prompt(&mut receive, "a removed profile stays quiet until Connect").await;
    calls.lock().unwrap().clear();
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
        vec![("owner.profileUnpair".into(), json!({"profileId":profile}))],
        "Connect only lets the helper ask again"
    );
    notify(&core, REQUESTS[2], &profile);
    let (request, answer) = prompt(&mut receive).await;
    assert!(
        request.summary.starts_with("Connect Chrome"),
        "{}",
        request.summary
    );
    answer.send(Confirmation::Declined).unwrap();
    core.shutdown().await;
}

#[tokio::test]
async fn a_profile_that_was_never_removed_still_asks_at_once() {
    let (core, mut receive) = start("rmquiet-new").await;
    let removed = format!("chrome:{PROFILE}");
    let fresh = format!("chrome:{OTHER}");
    connected(&core, &[&removed]);
    let _calls = answering(&core);
    remove_row(&core, &removed).await;
    notify(&core, REQUESTS[0], &fresh);
    let (request, answer) = prompt(&mut receive).await;
    assert!(
        request.summary.starts_with("Connect Chrome"),
        "{}",
        request.summary
    );
    answer.send(Confirmation::Declined).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let view = card(&core, Browser::Chrome);
    assert!(
        view.step
            .as_deref()
            .unwrap_or_default()
            .contains("you chose Not now"),
        "a person's own Not now keeps its text: {view:?}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn removing_the_whole_chrome_card_keeps_its_profiles_quiet_until_connect() {
    let (core, mut receive) = start("rmquiet-card").await;
    let profile = format!("chrome:{PROFILE}");
    connected(&core, &[&profile]);
    let _calls = answering(&core);
    core.chrome_control(crate::Origin::Ui, super::ChromeControlRequest::Remove {})
        .await
        .unwrap();
    notify(&core, REQUESTS[0], &profile);
    no_prompt(
        &mut receive,
        "a profile of a removed browser must not ask by itself",
    )
    .await;
    // Connect starts setup again (it may stop early without a package here); its next request asks.
    let _ = core
        .chrome_control(crate::Origin::Ui, super::ChromeControlRequest::Connect {})
        .await;
    notify(&core, REQUESTS[1], &profile);
    let (request, answer) = prompt(&mut receive).await;
    assert!(
        request.summary.starts_with("Connect Chrome"),
        "{}",
        request.summary
    );
    answer.send(Confirmation::Declined).unwrap();
    core.shutdown().await;
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn a_removed_edge_profile_does_not_ask_by_itself() {
    let (core, mut receive) = start("rmquiet-edge").await;
    core.shared
        .chrome_control
        .set_browser_enabled(Browser::Edge, true)
        .unwrap();
    let profile = format!("edge:{PROFILE}");
    connected(&core, &[&profile]);
    let calls = answering(&core);
    remove_row(&core, &profile).await;
    notify(&core, REQUESTS[0], &profile);
    no_prompt(
        &mut receive,
        "a removed Edge profile must not ask by itself",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        declined_quietly(&calls, REQUESTS[0]),
        "{:?}",
        calls.lock().unwrap()
    );
    let view = card(&core, Browser::Edge);
    assert_eq!(
        view.step.as_deref(),
        Some(
            "You removed an Edge profile that is still connected in Edge. Click Connect Edge to pair it again."
        ),
        "{view:?}"
    );
    core.shutdown().await;
}

/// Only the person's Connect lets a removed or declined profile ask again: a Debug API Connect leaves the quiet in place.
///
/// PREDICT (red before the change): the Debug API's Connect ran the same "ask again" step as the person's, sent
/// `owner.profileUnpair` for the waiting profile and ended the removal quiet, so the next reconnect opened the dialog.
#[tokio::test]
async fn a_debug_api_connect_never_lets_a_removed_profile_ask_again() {
    let (core, mut receive) = start("pairing-removed-debug-api").await;
    let calls = answering(&core);
    connected(&core, &[PROFILE]);
    remove_row(&core, PROFILE).await;
    notify(&core, REQUESTS[0], PROFILE);
    no_prompt(&mut receive, "the reconnect after Remove stays quiet").await;
    let unpairs = |calls: &Calls| {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "owner.profileUnpair")
            .count()
    };
    let before = unpairs(&calls);
    let _ = core
        .chrome_control(
            crate::Origin::DebugApi,
            super::ChromeControlRequest::Connect {},
        )
        .await;
    assert_eq!(
        unpairs(&calls),
        before,
        "a Debug API Connect must not ask the helper to forget the answer"
    );
    notify(&core, REQUESTS[1], PROFILE);
    no_prompt(
        &mut receive,
        "after a Debug API Connect the removed profile is still quiet",
    )
    .await;
    assert!(declined_quietly(&calls, REQUESTS[1]));
}
