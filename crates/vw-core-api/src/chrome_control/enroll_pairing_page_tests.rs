//! Open pairing page while Chrome's extension holds another Vibe Work: the connected extension opens its own page
//! in its own profile's window through the helper; without one, the browser launch stays the route. Uses the
//! enrollment rig (offline core, recording launcher, scripted owner).
//!
//! PREDICT (before the helper request): every case launched the browser with the plain page, so with two profiles
//! open the page landed in whichever profile the browser used last, not the one paired with another Vibe Work.
use super::*;

const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn mismatch(rig: &Rig) {
    owner::observe(
        &Arc::downgrade(&rig.core.shared),
        &json!({"method":"notifications/owner/pairing","params":{"profileId":PROFILE,"state":"mismatch","fingerprint":OTHER}}),
    )
    .unwrap();
}

fn page_requests(rig: &Rig) -> Vec<Value> {
    rig.calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == enroll::OPEN_PAIRING_PAGE)
        .map(|(_, args)| args.clone())
        .collect()
}

fn answering(open: Value) -> impl Fn(&str, &Value) -> Value + Send + 'static {
    move |name, args| {
        if name == enroll::OPEN_PAIRING_PAGE {
            open.clone()
        } else {
            ticket_answer(name, args)
        }
    }
}

async fn settle_long(rig: &Rig) {
    for _ in 0..10 {
        rig.settle().await;
    }
}

#[tokio::test]
async fn open_pairing_page_asks_the_extension_paired_elsewhere_and_launches_nothing() {
    let rig = Rig::new("chrome-pairing-page-asked", answering(json!({"asked":1}))).await;
    mismatch(&rig);
    let card = addon(&rig);
    assert_eq!(card.state, AddonState::OtherOwner);
    assert_eq!(card.action, Some(AddonAction::OpenPairingPage));
    rig.connect(Origin::Ui).await;
    settle_long(&rig).await;
    assert_eq!(page_requests(&rig), [json!({"browser":"chrome"})]);
    assert_eq!(
        rig.opened(),
        Vec::<String>::new(),
        "the extension opened its own page"
    );
    assert!(!has_attempt(&rig));
    rig.core.shutdown().await;
}

#[tokio::test]
async fn without_an_extension_that_can_open_it_the_browser_opens_the_page_as_before() {
    for (label, open) in [
        ("unsupported", json!({"none":"unsupported"})),
        ("none", json!({"none":"no_connection"})),
        ("older-helper", json!({"rpcError":{"code":-32601}})),
    ] {
        let rig = Rig::new(&format!("chrome-pairing-page-{label}"), answering(open)).await;
        mismatch(&rig);
        rig.connect(Origin::Ui).await;
        settle_long(&rig).await;
        assert_eq!(page_requests(&rig).len(), 1, "{label}");
        assert_eq!(rig.opened(), [plain()], "{label}");
        rig.core.shutdown().await;
    }
}

#[tokio::test]
async fn a_mismatch_report_after_connect_asks_that_extension() {
    for (asked, launched) in [(1, Vec::<String>::new()), (0, vec![plain()])] {
        let open = if asked == 1 {
            json!({"asked":1})
        } else {
            json!({"none":"unsupported"})
        };
        let rig = Rig::new(
            &format!("chrome-pairing-page-late-{asked}"),
            answering(open),
        )
        .await;
        rig.connect(Origin::Ui).await;
        let mut expected = rig.opened();
        expected.extend(launched);
        mismatch(&rig);
        settle_long(&rig).await;
        assert_eq!(page_requests(&rig), [json!({"browser":"chrome"})]);
        assert_eq!(rig.opened(), expected);
        rig.core.shutdown().await;
    }
}

#[tokio::test]
async fn a_waiting_extension_keeps_the_ticket_route() {
    let rig = Rig::new("chrome-pairing-page-waiting", answering(json!({"asked":1}))).await;
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    settle_long(&rig).await;
    assert_eq!(page_requests(&rig), Vec::<Value>::new());
    assert_eq!(rig.opened(), [enroll_url()]);
    rig.core.shutdown().await;
}
