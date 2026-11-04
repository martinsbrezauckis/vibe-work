//! Scoped helper profiles must reach the correct browser with the extension's raw UUID in the fragment.
//! Uses the enrollment rig's fake RPC answers and separate recording launchers; called by enrollment unit tests.
use super::*;

fn report(rig: &Rig, profile: &str, auto: bool) {
    owner::observe(
        &Arc::downgrade(&rig.core.shared),
        &json!({"method":"notifications/owner/pairing", "params":{
            "profileId":profile,"state":"waiting","fingerprint":OURS,"enroll":u8::from(auto)
        }}),
    )
    .unwrap();
}

fn scoped_ticket(name: &str, _: &Value) -> Value {
    if name == enroll::MINT {
        json!({"ticket":TICKET,"profileId":format!("chrome:{PROFILE}"),"fingerprint":OURS})
    } else {
        json!({})
    }
}

#[tokio::test]
async fn scoped_chrome_ticket_opens_enrollment_with_the_raw_profile() {
    let rig = Rig::new("chrome-scoped-mint", scoped_ticket).await;
    report(&rig, &format!("chrome:{PROFILE}"), true);
    rig.connect(Origin::Ui).await;
    assert_eq!(
        rig.opened(),
        [enroll_url()],
        "a scoped Chrome ticket fell back to Pair"
    );
    assert!(rig.edge_opened.lock().unwrap().is_empty());
    rig.core.shutdown().await;
}

#[test]
fn chrome_mint_rejects_other_browsers_and_malformed_scopes() {
    for profile in [
        format!("edge:{PROFILE}"),
        format!("firefox:{PROFILE}"),
        format!("Chrome:{PROFILE}"),
        format!("chrome:chrome:{PROFILE}"),
        format!("chrome:{PROFILE}/extra"),
        "chrome:bad".to_owned(),
    ] {
        let answer = json!({"ticket":TICKET,"profileId":profile,"fingerprint":OURS});
        assert_eq!(
            super::super::minted(Some(&answer), Some(OURS), enroll_browser::Browser::Chrome),
            super::super::Minted::Other
        );
    }
}

#[test]
fn edge_mint_rejects_legacy_chrome_profiles_and_other_browser_scopes() {
    use super::super::{Minted, minted};
    for profile in [
        PROFILE.to_owned(),
        format!("chrome:{PROFILE}"),
        format!("firefox:{PROFILE}"),
        format!("edge:edge:{PROFILE}"),
        "edge:bad".to_owned(),
    ] {
        let answer = json!({"ticket":TICKET,"profileId":profile,"fingerprint":OURS});
        assert_eq!(
            minted(Some(&answer), Some(OURS), enroll_browser::Browser::Edge),
            Minted::Other
        );
    }
    let answer = json!({"ticket":TICKET,"profileId":format!("edge:{PROFILE}"),"fingerprint":OURS});
    assert!(
        matches!(minted(Some(&answer), Some(OURS), enroll_browser::Browser::Edge),
        Minted::Ticket {profile, ..} if profile == PROFILE)
    );
}

#[tokio::test]
async fn foreign_status_profiles_never_pair_chrome_in_a_chrome_only_app() {
    let rig = Rig::new("chrome-scoped-status", ticket_answer).await;
    owner::update_pairing(
        &Arc::downgrade(&rig.core.shared),
        &json!({"structuredContent":{
            "profiles":[{"profileId":format!("edge:{PROFILE}"),"connected":true}]
        }}),
    );
    assert_ne!(rig.core.chrome_view().setup.pairing, "Paired");
    rig.core.shutdown().await;
}

#[tokio::test]
async fn other_browser_reports_never_consume_a_chrome_connect_even_without_browser_features() {
    let rig = Rig::new("chrome-scoped-isolation", scoped_ticket).await;
    rig.connect(Origin::Ui).await;
    let before = rig.opened();
    for profile in [format!("edge:{PROFILE}"), format!("firefox:{PROFILE}")] {
        report(&rig, &profile, true);
        rig.settle().await;
        assert_eq!(
            rig.opened(),
            before,
            "another browser consumed Chrome's Connect"
        );
    }
    report(&rig, &format!("chrome:{PROFILE}"), true);
    rig.settle().await;
    assert_eq!(rig.opened().last(), Some(&enroll_url()));
    rig.core.shutdown().await;
}

#[cfg(feature = "your-edge")]
async fn connect_edge(rig: &Rig, origin: Origin) {
    rig.core
        .chrome_control(origin, ChromeControlRequest::OpenEdgeStore {})
        .await
        .unwrap();
}

#[cfg(feature = "your-edge")]
#[tokio::test(start_paused = true)]
async fn expired_edge_attempt_opens_its_expired_pair_page_once() {
    for seconds in [600, 601] {
        let rig = Rig::new(&format!("edge-scoped-expired-{seconds}"), ticket_answer).await;
        connect_edge(&rig, Origin::Ui).await;
        let mut expected = rig.edge_opened.lock().unwrap().clone();
        expected.push(expired_url());
        tokio::time::advance(std::time::Duration::from_secs(seconds)).await;
        for _ in 0..2 {
            report(&rig, &format!("edge:{PROFILE}"), true);
            rig.settle().await;
            assert_eq!(*rig.edge_opened.lock().unwrap(), expected);
            assert_eq!(rig.opened(), Vec::<String>::new());
            assert_eq!(rig.methods(), [enroll::REVOKE]);
            assert!(
                rig.core
                    .shared
                    .chrome_control
                    .owner
                    .edge_enroll
                    .lock()
                    .unwrap()
                    .attempt
                    .is_none()
            );
        }
        rig.core.shutdown().await;
    }
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn edge_store_extension_waiting_opens_its_pair_page_once() {
    let rig = Rig::new("edge-scoped-plain", ticket_answer).await;
    connect_edge(&rig, Origin::Ui).await;
    let store = setup::store_url(setup::EXTENSION_ID);
    assert_eq!(
        rig.edge_opened.lock().unwrap().as_slice(),
        std::slice::from_ref(&store)
    );
    report(&rig, &format!("chrome:{PROFILE}"), false);
    assert_eq!(
        rig.edge_opened.lock().unwrap().as_slice(),
        std::slice::from_ref(&store)
    );
    report(&rig, &format!("edge:{PROFILE}"), false);
    report(&rig, &format!("edge:{PROFILE}"), false);
    assert_eq!(*rig.edge_opened.lock().unwrap(), [store, plain()]);
    assert_eq!(rig.opened(), Vec::<String>::new());
    assert!(!rig.methods().iter().any(|m| m == enroll::MINT));
    rig.core.shutdown().await;
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn edge_scoped_ticket_opens_only_edge_and_only_for_a_window_connect() {
    let rig = Rig::new("edge-scoped-mint", |name, _| {
        if name == enroll::MINT {
            json!({"ticket":TICKET,"profileId":format!("edge:{PROFILE}"),"fingerprint":OURS})
        } else {
            json!({})
        }
    })
    .await;
    connect_edge(&rig, Origin::Ui).await;
    report(&rig, &format!("chrome:{PROFILE}"), true);
    report(&rig, &format!("edge:{PROFILE}"), true);
    rig.settle().await;
    assert_eq!(rig.edge_opened.lock().unwrap().last(), Some(&enroll_url()));
    assert_eq!(rig.opened(), Vec::<String>::new());
    connect_edge(&rig, Origin::DebugApi).await;
    report(&rig, &format!("edge:{PROFILE}"), true);
    rig.settle().await;
    assert_eq!(rig.edge_opened.lock().unwrap().last(), Some(&plain()));
    assert_eq!(
        rig.methods().iter().filter(|m| *m == enroll::MINT).count(),
        1
    );
    rig.core.shutdown().await;
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn edge_connect_refuses_a_chrome_ticket() {
    let rig = Rig::new("edge-scoped-refuse-chrome", scoped_ticket).await;
    connect_edge(&rig, Origin::Ui).await;
    report(&rig, &format!("edge:{PROFILE}"), true);
    rig.settle().await;
    assert_eq!(rig.edge_opened.lock().unwrap().last(), Some(&plain()));
    assert_eq!(rig.opened(), Vec::<String>::new());
    rig.core.shutdown().await;
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn adding_edge_to_existing_setup_opens_its_pair_page_after_get() {
    let rig = Rig::new("edge-scoped-setup", ticket_answer).await;
    let chrome = &rig.core.shared.chrome_control;
    let dir = chrome.dir().unwrap();
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(dir.join("bin").join(setup::helper_name()), b"test helper").unwrap();
    let crate::CommandResult::ChromeControl {
        offer: Some(offer), ..
    } = rig
        .core
        .chrome_control(Origin::Ui, ChromeControlRequest::OfferEdgeSetup {})
        .await
        .unwrap()
    else {
        panic!("Edge offer")
    };
    rig.core
        .chrome_control(
            Origin::Ui,
            ChromeControlRequest::SetupEdge {
                offer_id: offer.offer_id,
            },
        )
        .await
        .unwrap();
    report(&rig, &format!("edge:{PROFILE}"), false);
    assert_eq!(rig.edge_opened.lock().unwrap().last(), Some(&plain()));
    assert_eq!(rig.opened(), Vec::<String>::new());
    rig.core.shutdown().await;
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn a_new_browser_connect_ends_the_other_browsers_attempt() {
    let rig = Rig::new("edge-scoped-replace", ticket_answer).await;
    rig.connect(Origin::Ui).await;
    assert!(super::has_attempt(&rig));
    connect_edge(&rig, Origin::Ui).await;
    assert!(!super::has_attempt(&rig));
    assert!(
        !rig.core
            .shared
            .chrome_control
            .connect_requested
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    rig.core.shared.chrome_control.owner.stop().await;
    assert!(
        rig.core
            .shared
            .chrome_control
            .owner
            .edge_enroll
            .lock()
            .unwrap()
            .attempt
            .is_none()
    );
    rig.core.shutdown().await;
}
