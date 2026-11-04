//! Captures enrollment decisions with a fake helper and launcher; checks useful labels and secret exclusion.
//! Depends on the enrollment test rig and tracing, with a thread-local subscriber and no global logger changes.
use super::*;
use std::fmt::Write;
use tracing::{Event, Metadata, Subscriber, field::Visit, span};

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<String>>);

struct Capture {
    _thread: tracing::subscriber::DefaultGuard,
    _uncaptured: tracing::Dispatch,
}

impl Logs {
    fn capture(&self) -> Capture {
        // With only one registered dispatch, tracing caches a new callsite's interest from the calling
        // thread. A parallel test without a subscriber can then disable that callsite for this capture.
        // Keep an inert second dispatch alive so registration consults both subscribers instead.
        let uncaptured = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        Capture {
            _thread: tracing::subscriber::set_default(self.clone()),
            _uncaptured: uncaptured,
        }
    }
}

struct Fields<'a>(&'a mut String);

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        write!(self.0, " {}={value:?}", field.name()).unwrap();
    }
}

#[test]
fn callsite_first_seen_on_uncaptured_thread_is_still_logged() {
    fn decision() {
        tracing::info!("Chrome enrollment capture probe");
    }

    let logs = Logs::default();
    let _capture = logs.capture();
    std::thread::spawn(decision).join().unwrap();
    assert!(logs.0.lock().unwrap().is_empty());
    decision();
    assert_eq!(
        logs.0.lock().unwrap().as_str(),
        " message=Chrome enrollment capture probe\n"
    );
}

#[tokio::test(start_paused = true)]
async fn waiting_retry_and_expiry_decisions_are_logged() {
    let logs = Logs::default();
    let _capture = logs.capture();
    let rig = Rig::new("chrome-enroll-log-waiting", |name, _| {
        if name == enroll::MINT {
            json!({"none":"no_waiting"})
        } else {
            json!({})
        }
    })
    .await;
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    rig.waiting(true);
    rig.settle().await;
    assert_eq!(rig.opened(), [plain()]);
    rig.connect(Origin::Ui).await;
    tokio::time::advance(enroll::ATTEMPT_LIFE + std::time::Duration::from_secs(1)).await;
    rig.waiting(true);
    rig.settle().await;
    assert_eq!(rig.opened().last(), Some(&expired_url()));
    rig.core.shutdown().await;
    let output = logs.0.lock().unwrap();
    for expected in [
        "Chrome enrollment on_waiting",
        "Chrome enrollment take_mint_on_waiting",
        "mint_on_waiting=true",
        "attempt_age=Some(601s)",
        "NoWaiting",
        "caller=\"on_waiting_no_waiting\"",
    ] {
        assert!(output.contains(expected), "missing {expected}: {output}");
    }
    for secret in [TICKET, PROFILE, OURS] {
        assert!(
            !output.contains(secret),
            "enrollment secret appeared in tracing output"
        );
    }
}

impl Subscriber for Logs {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut output = self.0.lock().unwrap();
        event.record(&mut Fields(&mut output));
        output.push('\n');
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

#[tokio::test]
async fn decisions_are_logged_without_enrollment_secrets() {
    let logs = Logs::default();
    let _capture = logs.capture();
    let rig = Rig::new("chrome-enroll-log", ticket_answer).await;
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    assert_eq!(rig.opened(), [enroll_url()]);
    let attempt = rig
        .calls
        .lock()
        .unwrap()
        .iter()
        .find(|(name, _)| name == enroll::MINT)
        .unwrap()
        .1["attempt"]
        .as_str()
        .unwrap()
        .to_owned();
    rig.connect(Origin::DebugApi).await;
    rig.core.set_window_drive(|| true);
    rig.connect(Origin::Ui).await;
    rig.core.shutdown().await;

    // Untrusted state/none words must be reduced to fixed labels, even if they contain a secret or URL.
    let hostile = format!("{TICKET}{PROFILE}{OURS}{attempt}{}", enroll_url());
    let mut state = super::super::Enroll::default();
    state.observe_state(
        Some(&hostile),
        &json!({"enroll":1,"fingerprint":OURS}),
        enroll_browser::Browser::Chrome,
    );
    state.observe_state(None, &json!({}), enroll_browser::Browser::Chrome);
    for bad in [
        json!({"none":hostile}),
        json!({"none":"ambiguous"}),
        json!({"ticket":hostile,"profileId":PROFILE,"fingerprint":OURS}),
        json!({"ticket":TICKET,"profileId":hostile,"fingerprint":OURS}),
        json!({"ticket":TICKET,"profileId":PROFILE,"fingerprint":hostile}),
        json!({"ticket":TICKET,"profileId":PROFILE,"fingerprint":"b".repeat(64)}),
    ] {
        assert_eq!(
            super::super::minted(Some(&bad), Some(OURS), enroll_browser::Browser::Chrome),
            super::super::Minted::Other
        );
    }
    assert_eq!(
        super::super::minted(None, None, enroll_browser::Browser::Chrome),
        super::super::Minted::Other
    );

    let output = logs.0.lock().unwrap();
    for secret in [
        TICKET,
        PROFILE,
        OURS,
        &attempt,
        &enroll_url(),
        &"b".repeat(64),
    ] {
        assert!(
            !output.contains(secret),
            "enrollment secret appeared in tracing output"
        );
    }
    for decision in [
        "begin",
        "observe_state",
        "connection_ended",
        "open_for_waiting",
        "mint_and_open",
        "open_plain",
    ] {
        assert!(
            output.contains(&format!("Chrome enrollment {decision}")),
            "missing {decision}: {output}"
        );
    }
    for label in [
        "Ticket",
        "Other",
        "debug_api_origin",
        "window_drive",
        "ambiguous",
        "unknown",
        "ticket_shape",
        "profile_shape",
        "fingerprint_shape",
        "fingerprint_differs",
        "no_answer",
    ] {
        assert!(output.contains(label), "missing {label}: {output}");
    }
}
