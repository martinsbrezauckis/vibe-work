//! Setting a browser up again after its whole card was removed lets its profiles ask to connect.
//! Uses the offline core, a scripted private owner and a controlled native presenter; no desktop or real browser.
//!
//! PREDICT (red before the change): Remove on the whole Chrome card deletes its setup, so Settings' next Connect
//! Chrome runs the setup (offer, then `setup`) instead of the plain Connect. Only the plain Connect ended the removed
//! profiles' quiet; after the setup the extension's Replace raised a pairing request that Vibe Work answered "Not now"
//! without a dialog, and the extension showed "This profile needs approval in Vibe Work" with no question to answer
//! (until Vibe Work restarted). The test fails on "the request must reach the native presenter". After: the person's
//! setup ends the quiet like Connect, so the next request asks; a Debug API setup keeps the quiet.
use std::{sync::Arc, time::Duration};

use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

const PROFILE: &str = "chrome:66666666-6666-4666-8666-666666666666";
const REQUESTS: [&str; 2] = [
    "77777777-7777-4777-8777-777777777777",
    "88888888-8888-4888-8888-888888888888",
];

type Prompts = mpsc::UnboundedReceiver<(ConfirmationRequest, oneshot::Sender<Confirmation>)>;

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

/// Chrome set up with a packaged helper, one connected profile, then the whole Chrome card removed in Settings.
#[allow(clippy::unwrap_used, reason = "isolated test fixture initialization")]
async fn removed_chrome(name: &str) -> (crate::Core, Prompts) {
    let (send, receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        name,
        |config, dir| {
            let package = dir.join("package");
            std::fs::create_dir_all(package.join("bin")).unwrap();
            std::fs::write(
                package.join("bin").join(super::setup::helper_name()),
                b"helper",
            )
            .unwrap();
            config.chrome_control.package = Some(package);
            config.chrome_control.user_home = Some(dir.join("home"));
        },
        Presenter(send),
    )
    .await;
    let chrome = &core.shared.chrome_control;
    chrome.set_enabled(true).unwrap();
    chrome.owner.answering(|tool, _| match tool {
        "owner.profileUnpair" | "owner.browserUnpair" => json!({"unpaired":true}),
        "owner.pairing.approve" => json!({"approved":false}),
        _ => json!({"applied":true}),
    });
    super::test_support::observe_status(
        &core.shared,
        &json!({"profiles":[{"profileId":PROFILE,"connected":true}]}),
    );
    core.chrome_control(crate::Origin::Ui, super::ChromeControlRequest::Remove {})
        .await
        .unwrap();
    (core, receive)
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

/// Settings' Connect Chrome after a whole-card Remove: the setup offer, then `setup` (it may stop early here, without
/// a keyring or a real browser; the person's choice to connect is what counts).
#[allow(
    clippy::unwrap_used,
    reason = "the offer is made for a packaged helper"
)]
async fn set_up_again(core: &crate::Core, origin: crate::Origin) {
    let offer = core
        .chrome_control(origin, super::ChromeControlRequest::OfferSetup {})
        .await
        .unwrap();
    let crate::CommandResult::ChromeControl {
        offer: Some(offer), ..
    } = offer
    else {
        panic!("the setup offer is made");
    };
    let _ = core
        .chrome_control(
            origin,
            super::ChromeControlRequest::Setup {
                offer_id: offer.offer_id.clone(),
            },
        )
        .await;
}

#[tokio::test]
async fn setting_chrome_up_again_after_removing_it_lets_its_profile_ask() {
    let (core, mut receive) = removed_chrome("pairing-removed-setup").await;
    notify(&core, REQUESTS[0]);
    let quiet = tokio::time::timeout(Duration::from_millis(300), receive.recv()).await;
    assert!(quiet.is_err(), "a removed browser's profile stays quiet");
    set_up_again(&core, crate::Origin::Ui).await;
    notify(&core, REQUESTS[1]);
    let (request, answer) = tokio::time::timeout(Duration::from_secs(2), receive.recv())
        .await
        .expect("the request must reach the native presenter")
        .expect("the presenter is open");
    assert!(
        request.summary.starts_with("Connect Chrome"),
        "{}",
        request.summary
    );
    let _ = answer.send(Confirmation::Declined);
    core.shutdown().await;
}

#[tokio::test]
async fn a_debug_api_setup_keeps_a_removed_profile_quiet() {
    let (core, mut receive) = removed_chrome("pairing-removed-setup-debug").await;
    set_up_again(&core, crate::Origin::DebugApi).await;
    notify(&core, REQUESTS[1]);
    let quiet = tokio::time::timeout(Duration::from_millis(300), receive.recv()).await;
    assert!(
        quiet.is_err(),
        "only the person's own setup lets a removed profile ask again"
    );
    core.shutdown().await;
}
