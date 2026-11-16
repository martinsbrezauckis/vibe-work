//! A browser profile the person removes in Settings does not ask to connect again by itself.
//! Uses the offline core, a scripted private owner and a controlled native presenter; no desktop or real browser.
//!
//! PREDICT (red before the change): the extension of a removed profile that is still installed reconnects about a
//! second after Remove, the helper raises a new pairing request, and Vibe Work opened "Connect Chrome profile … to
//! Vibe Work?" although the person just removed it and pressed nothing. Now Remove (row or whole browser card) marks
//! the identity for this app run: its requests are answered "Not now" without a dialog, the card says the removed
//! profile is still connected in Chrome and offers Connect, and only that Connect lets the next request ask. A profile
//! that was never removed still asks at once.
//!
//! PREDICT (label): the removed profile's card read "Waiting for your approval", the Not now card's label, although the
//! person removed it and no approval waits. It now reads "Not connected" with the same step and Connect.
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

/// Waits until the "Not now" for `profile` is recorded. That happens only after the helper answered
/// `owner.pairing.approve` (`pairing_approval::answered`, in a spawned task), so a fixed 50 ms sleep raced it: under
/// the full gate's load Connect ran first and sent nothing.
async fn not_now_recorded(core: &crate::Core, profile: &str) {
    let approvals = &core.shared.chrome_control.owner.approvals;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !approvals
        .declined_for(Browser::from_profile(profile))
        .iter()
        .any(|p| p == profile)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the Not now for {profile} was never recorded"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
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
    let (core, mut receive) = start("pairing-removed-row").await;
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
    not_now_recorded(&core, &profile).await;
    assert!(
        declined_quietly(&calls, REQUESTS[0]),
        "the helper hears Not now so it stops asking: {:?}",
        calls.lock().unwrap()
    );
    // The person removed the profile; nothing waits for their approval, so the card keeps a plain "Not connected"
    // label ("Waiting for your approval" stays the Not now card's).
    let view = card(&core, Browser::Chrome);
    assert_eq!(view.label, "Not connected", "{view:?}");
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
        vec![(
            "owner.profileUnpair".into(),
            json!({"profileId":profile,"keepPin":true})
        )],
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
    let (core, mut receive) = start("pairing-removed-new").await;
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
    not_now_recorded(&core, &fresh).await;
    let view = card(&core, Browser::Chrome);
    assert!(
        view.step
            .as_deref()
            .unwrap_or_default()
            .contains("you chose Not now"),
        "a person's own Not now keeps its text: {view:?}"
    );
    assert_eq!(view.label, "Waiting for your approval", "{view:?}");
    core.shutdown().await;
}

#[tokio::test]
async fn removing_the_whole_chrome_card_keeps_its_profiles_quiet_until_connect() {
    let (core, mut receive) = start("pairing-removed-card").await;
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
    let (core, mut receive) = start("pairing-removed-edge").await;
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
    not_now_recorded(&core, &profile).await;
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
    core.shutdown().await;
}

fn unpaired(calls: &Calls) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(tool, _)| tool == "owner.profileUnpair")
        .count()
}

/// The person's "Not now" is kept the same way: a Debug API Connect, or a window Connect sent while the Debug API drives
/// the window, does not make the helper forget it; the person's own Connect then asks again.
///
/// PREDICT (written before any change; the code already gates both): green. With the `debug_api_acts` gate removed
/// from `ask_again` both driven Connects send `owner.profileUnpair` and the test fails.
#[tokio::test]
async fn not_now_survives_a_debug_api_and_a_window_driven_connect() {
    let (core, mut receive) = start("pairing-removed-not-now").await;
    let drive = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let probe = Arc::clone(&drive);
    core.set_window_drive(move || probe.load(std::sync::atomic::Ordering::SeqCst));
    let profile = format!("chrome:{OTHER}");
    let calls = answering(&core);
    notify(&core, REQUESTS[0], &profile);
    let (_, answer) = prompt(&mut receive).await;
    answer.send(Confirmation::Declined).unwrap();
    not_now_recorded(&core, &profile).await;
    let connect = |origin| core.chrome_control(origin, super::ChromeControlRequest::Connect {});
    let _ = connect(crate::Origin::DebugApi).await;
    assert_eq!(unpaired(&calls), 0, "{:?}", calls.lock().unwrap());
    drive.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = connect(crate::Origin::Ui).await;
    assert_eq!(unpaired(&calls), 0, "{:?}", calls.lock().unwrap());
    drive.store(false, std::sync::atomic::Ordering::SeqCst);
    let own = connect(crate::Origin::Ui).await.unwrap();
    let crate::CommandResult::ChromeControl { hint, .. } = own else {
        panic!("Connect answers with the browser view");
    };
    assert!(hint.unwrap_or_default().contains("Check connection"));
    assert_eq!(unpaired(&calls), 1, "{:?}", calls.lock().unwrap());
    core.shutdown().await;
}

/// A removed Edge profile stays quiet through the Debug API's Edge Connect (`OpenEdgeStore`).
///
/// PREDICT (written before any change; the code already gates it): green. Without the gate the Debug API's Edge
/// Connect forgets the removal and the next reconnect opens the dialog.
#[cfg(feature = "your-edge")]
#[tokio::test]
async fn a_debug_api_edge_connect_keeps_a_removed_edge_profile_quiet() {
    let (core, mut receive) = start("pairing-removed-edge-debug-api").await;
    core.shared
        .chrome_control
        .set_browser_enabled(Browser::Edge, true)
        .unwrap();
    let profile = format!("edge:{PROFILE}");
    connected(&core, &[&profile]);
    let calls = answering(&core);
    remove_row(&core, &profile).await;
    calls.lock().unwrap().clear();
    let _ = core
        .chrome_control(
            crate::Origin::DebugApi,
            super::ChromeControlRequest::OpenEdgeStore {},
        )
        .await;
    notify(&core, REQUESTS[0], &profile);
    no_prompt(&mut receive, "a Debug API Edge Connect keeps the quiet").await;
    not_now_recorded(&core, &profile).await;
    assert!(declined_quietly(&calls, REQUESTS[0]));
    assert_eq!(unpaired(&calls), 0, "{:?}", calls.lock().unwrap());
    core.shutdown().await;
}

/// The person's Connect after "Not now" asks the helper to keep a saved pin (`keepPin`): the helper decides from its
/// own pins, so a pin Vibe Work holds no record of is never dropped by asking again.
///
/// PREDICT: red before `keepPin` — Connect sent only `{"profileId"}`, the plain removal that drops a helper pin.
#[tokio::test]
async fn connect_after_not_now_asks_the_helper_to_keep_a_saved_pin() {
    let (core, mut receive) = start("pairing-removed-keep-pin").await;
    let profile = format!("chrome:{OTHER}");
    let calls = answering(&core);
    notify(&core, REQUESTS[0], &profile);
    let (_, answer) = prompt(&mut receive).await;
    answer.send(Confirmation::Declined).unwrap();
    not_now_recorded(&core, &profile).await;
    let _ = core
        .chrome_control(crate::Origin::Ui, super::ChromeControlRequest::Connect {})
        .await;
    let sent: Vec<Value> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(tool, _)| tool == "owner.profileUnpair")
        .map(|(_, args)| args.clone())
        .collect();
    assert_eq!(sent, [json!({"profileId":profile,"keepPin":true})]);
    notify(&core, REQUESTS[1], &profile);
    let _ = prompt(&mut receive).await;
    core.shutdown().await;
}

/// An older helper refuses the `keepPin` field: Connect then sends the plain request it always sent, so asking
/// again still works.
///
/// PREDICT: red before the fallback — with only the `keepPin` request the older helper's refusal left the "Not now"
/// in place and the next request opened no dialog.
#[tokio::test]
async fn connect_after_not_now_falls_back_for_an_older_helper() {
    let (core, mut receive) = start("pairing-removed-keep-pin-old").await;
    let profile = format!("chrome:{OTHER}");
    let calls = core
        .shared
        .chrome_control
        .owner
        .answering(|tool, args| match tool {
            "owner.profileUnpair" if args.get("keepPin").is_some() => {
                json!({"rpcError":{"code":-32602,"message":"Browser connection could not be removed"}})
            }
            "owner.profileUnpair" => json!({"unpaired":true}),
            "owner.pairing.approve" => json!({"approved":false}),
            _ => json!({"applied":true}),
        });
    notify(&core, REQUESTS[0], &profile);
    let (_, answer) = prompt(&mut receive).await;
    answer.send(Confirmation::Declined).unwrap();
    not_now_recorded(&core, &profile).await;
    let _ = core
        .chrome_control(crate::Origin::Ui, super::ChromeControlRequest::Connect {})
        .await;
    let sent: Vec<Value> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(tool, _)| tool == "owner.profileUnpair")
        .map(|(_, args)| args.clone())
        .collect();
    assert_eq!(
        sent,
        [
            json!({"profileId":profile,"keepPin":true}),
            json!({"profileId":profile})
        ]
    );
    assert!(
        core.shared
            .chrome_control
            .owner
            .approvals
            .declined_for(Browser::Chrome)
            .is_empty(),
        "the older helper's plain removal forgets the Not now"
    );
    notify(&core, REQUESTS[1], &profile);
    let _ = prompt(&mut receive).await;
    core.shutdown().await;
}

// PREDICT (red before the change): two Chrome profiles were paired; the person removed the one whose extension was
// connected while the other stayed offline. The helper's status still listed the removed profile as connected, the
// card took that as the browser's own connection, and kept "Connected" ("Paired" on macOS) with the removed-profile
// step beside it, so the agent also read Your Chrome as connected. A profile the person removed is not this browser's
// connection: the card now reads "Not connected" with the same step and Connect, and the agent reads the same state.
#[tokio::test]
async fn removing_the_connected_profile_while_another_is_offline_reads_not_connected() {
    let (core, mut receive) = start("pairing-removed-connected").await;
    let removed = format!("chrome:{PROFILE}");
    let offline = format!("chrome:{OTHER}");
    // The packaged helper and Set up's record, with the same scripted helper answers as `answering`.
    let _calls = super::test_support::answering(&core.shared, |tool, _| match tool {
        "owner.profileUnpair" | "owner.browserUnpair" => json!({"unpaired":true}),
        "owner.pairing.approve" => json!({"approved":false}),
        _ => json!({"applied":true}),
    });
    let chrome = &core.shared.chrome_control;
    chrome
        .owner
        .tools
        .lock()
        .unwrap()
        .push(json!({"name":"chrome_status"}));
    // The extension is in a profile folder of a Chrome user data folder that Set up registered.
    let data = chrome
        .config
        .user_home
        .as_ref()
        .unwrap()
        .join("removed-connected-data");
    let folder = data.join("Default");
    std::fs::create_dir_all(
        folder
            .join("Extensions")
            .join(&chrome.config.extension_id)
            .join("0.1.12_0"),
    )
    .unwrap();
    std::fs::write(
        chrome.dir().unwrap().join("setup.json"),
        json!({"files":[],"browsers":["Google Chrome"],"userData":[data]}).to_string(),
    )
    .unwrap();
    let status = |removed_connected: bool| {
        super::test_support::observe_status(
            &core.shared,
            &json!({"profiles":[
                {"profileId":removed,"name":"Your Chrome","connected":removed_connected},
                {"profileId":offline,"name":"Work","connected":false}]}),
        );
    };
    status(true);
    assert_eq!(
        super::commands::addon(&core.shared).state,
        super::AddonState::Connected
    );
    remove_row(&core, &removed).await;
    // The removed extension asks again at once; it is answered Not now without a dialog.
    notify(&core, REQUESTS[0], &removed);
    no_prompt(&mut receive, "a removed profile does not ask by itself").await;
    not_now_recorded(&core, &removed).await;
    // The next status poll still lists the removed profile's extension as connected.
    status(true);
    let view = super::commands::addon(&core.shared);
    assert_ne!(view.state, super::AddonState::Connected, "{view:?}");
    assert_eq!(view.label, "Not connected", "{view:?}");
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
    assert!(
        !view.instruction.contains("is connected."),
        "the agent is not told the removed profile is connected: {}",
        view.instruction
    );
    core.shutdown().await;
}

/// The same rule for Edge: a removed Edge profile that the helper still lists as connected is not Edge's connection.
#[cfg(feature = "your-edge")]
#[tokio::test]
async fn a_removed_edge_profile_still_listed_as_connected_is_not_edges_connection() {
    let (core, _receive) = start("pairing-removed-edge-connected").await;
    core.shared
        .chrome_control
        .set_browser_enabled(Browser::Edge, true)
        .unwrap();
    let removed = format!("edge:{PROFILE}");
    let _calls = answering(&core);
    connected(&core, &[&removed]);
    let edge = || {
        core.shared
            .chrome_control
            .owner
            .edge_pairing
            .lock()
            .unwrap()
            .0
    };
    assert_eq!(edge(), super::PairingState::Paired);
    remove_row(&core, &removed).await;
    connected(&core, &[&removed]);
    assert_ne!(edge(), super::PairingState::Paired);
    core.shutdown().await;
}
