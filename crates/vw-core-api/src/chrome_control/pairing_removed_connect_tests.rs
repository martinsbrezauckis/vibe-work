//! Which Connect lets a removed or declined profile ask again: only the person's own Connect, never a Debug API or
//! window-driven one; it asks the helper to keep a saved pin and falls back for an older helper.
//! Uses the offline core, a scripted private owner and a controlled native presenter; no desktop or real browser.
use std::sync::Arc;

use serde_json::{Value, json};
use vw_gate::Confirmation;

use super::agent_browser::Browser;
use super::pairing_removed_tests::{
    Calls, OTHER, PROFILE, REQUESTS, answering, connected, declined_quietly, no_prompt,
    not_now_recorded, notify, prompt, remove_row, start,
};

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
