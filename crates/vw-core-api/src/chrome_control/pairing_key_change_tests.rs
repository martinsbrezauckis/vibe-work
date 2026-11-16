//! A paired browser profile whose key changed and that the person answered "Not now" can be asked again from Settings.
//! Uses the offline core, a scripted private owner and a controlled native presenter; no desktop or real browser.
//!
//! PREDICT (red before the change): the browser's stored extension key was lost, so the paired profile reconnected
//! with a new key and Vibe Work asked "Connect Chrome profile …?"; after "Not now" the card offered no Connect and
//! Connect did not ask the helper to forget the answer, because both skipped every identity Vibe Work holds as paired
//! (asking again with the plain removal would have ended its pin). Nothing asked again until Vibe Work restarted. The
//! test fails on the card's missing Connect action. After: the card offers Connect and Connect asks the helper to
//! forget only the answer while keeping the saved pin (`keepPin`); an older helper that refuses `keepPin` gets no
//! plain removal for a paired identity, so its pin is never ended by asking again.
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

use super::agent_browser::Browser;

const PROFILE: &str = "chrome:99999999-9999-4999-8999-999999999999";
const REQUESTS: [&str; 2] = [
    "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
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
        let _ = self.0.send((request.clone(), send));
        async move { receive.await.unwrap_or(Confirmation::Declined) }
    }
}

/// A core with `PROFILE` paired, whose helper answers `owner.profileUnpair` with `keep_pin` (a current helper) or
/// refuses `keepPin` (an older one).
#[allow(clippy::unwrap_used, reason = "isolated test fixture initialization")]
async fn paired(name: &str, keep_pin: bool) -> (crate::Core, Prompts, Calls) {
    let (send, receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        name,
        |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
        },
        Presenter(send),
    )
    .await;
    let chrome = &core.shared.chrome_control;
    chrome.set_enabled(true).unwrap();
    super::profiles::paired(chrome, PROFILE, None);
    let calls = chrome.owner.answering(move |tool, args| match tool {
        "owner.profileUnpair" if args.get("keepPin").is_some() && !keep_pin => {
            json!({"rpcError":{"code":-32602,"message":"Browser connection could not be removed"}})
        }
        "owner.profileUnpair" => json!({"unpaired":true,"pinKept":true}),
        "owner.pairing.approve" => json!({"approved":false}),
        _ => json!({"applied":true}),
    });
    (core, receive, calls)
}

#[allow(clippy::unwrap_used, reason = "the owner notification is well formed")]
fn notify(core: &crate::Core, id: &str) {
    super::owner::observe(
        &Arc::downgrade(&core.shared),
        &json!({"method":"notifications/owner/pairingApproval", "params":{
            "requestId":id,"profileId":PROFILE}}),
    )
    .unwrap();
}

async fn prompt(receive: &mut Prompts) -> oneshot::Sender<Confirmation> {
    tokio::time::timeout(Duration::from_secs(2), receive.recv())
        .await
        .expect("the request must reach the native presenter")
        .expect("the presenter is open")
        .1
}

/// The key-change question, answered "Not now"; returns once Vibe Work recorded the answer.
async fn declined_key_change(core: &crate::Core, receive: &mut Prompts) {
    notify(core, REQUESTS[0]);
    let _ = prompt(receive).await.send(Confirmation::Declined);
    let approvals = &core.shared.chrome_control.owner.approvals;
    tokio::time::timeout(Duration::from_secs(2), async {
        while approvals.declined_for(Browser::Chrome).is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the Not now is recorded");
}

fn card(core: &crate::Core) -> super::AddonView {
    let mut view = super::addon::view(super::AddonState::ChromeClosed, "");
    super::profile_reason::reconnecting(&core.shared, &mut view, true, Browser::Chrome);
    view
}

#[allow(clippy::unwrap_used, reason = "the recorded calls are not poisoned")]
fn unpair_requests(calls: &Calls) -> Vec<Value> {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(tool, _)| tool == "owner.profileUnpair")
        .map(|(_, args)| args.clone())
        .collect()
}

#[tokio::test]
async fn a_paired_profile_declined_after_a_key_change_can_be_asked_again() {
    let (core, mut receive, calls) = paired("pairing-key-change", true).await;
    declined_key_change(&core, &mut receive).await;
    assert_eq!(
        card(&core).action,
        Some(super::addon::AddonAction::Connect),
        "the card offers Connect to be asked again"
    );
    let _ = core
        .chrome_control(crate::Origin::Ui, super::ChromeControlRequest::Connect {})
        .await;
    assert_eq!(
        unpair_requests(&calls),
        [json!({"profileId":PROFILE,"keepPin":true})]
    );
    assert!(
        core.shared
            .chrome_control
            .profiles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .records
            .get(PROFILE)
            .is_some_and(|record| !record.removed),
        "asking again keeps the pairing"
    );
    notify(&core, REQUESTS[1]);
    let _ = prompt(&mut receive).await.send(Confirmation::Declined);
    core.shutdown().await;
}

#[tokio::test]
async fn an_older_helper_never_gets_the_plain_removal_for_a_paired_profile() {
    let (core, mut receive, calls) = paired("pairing-key-change-old", false).await;
    declined_key_change(&core, &mut receive).await;
    let _ = core
        .chrome_control(crate::Origin::Ui, super::ChromeControlRequest::Connect {})
        .await;
    assert_eq!(
        unpair_requests(&calls),
        [json!({"profileId":PROFILE,"keepPin":true})],
        "the plain removal would end the pin"
    );
    core.shutdown().await;
}
