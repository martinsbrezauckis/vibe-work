//! Chat work rows: release, stop, disconnect and the rows the Settings view lists.
use super::*;
use serde_json::json;

/// PREDICT: a disconnected row remains in `releases`, so selecting a new browser
/// tries to Stop a connection that no longer exists.
#[test]
fn disconnected_session_is_not_an_outstanding_release() {
    let mut work = Work::default();
    work.open("old", "chat", "active", "chrome:work", None);
    work.open("other", "other-chat", "active", "edge:work", None);
    work.observe(&[], &mut HashMap::new(), Instant::now());
    assert!(work.releases("chat").is_empty());
    assert!(work.releases("other-chat").is_empty());
}

#[test]
fn an_asked_stop_marks_only_the_owned_helper_session() {
    let mut work = Work::default();
    // Two sessions of one chat (Chrome and Edge) guard against using the chat-wide end path. Without Edge a
    // chat holds one live session per browser, so the second session exists only in the Edge build.
    let edge = cfg!(feature = "your-edge");
    work.open("a1", "a", "active", "chrome:p", None);
    if edge {
        work.open("a2", "a", "connecting", "edge:p", None);
    }
    work.open("b1", "b", "active", "chrome:p", None);
    work.ask_stop("a1", "b");
    work.ask_stop("unknown", "a");
    assert_eq!(work.state_of("a1", "a"), Some("active"));
    work.ask_stop("a1", "a");
    assert_eq!(work.state_of("a1", "a"), Some("stopping"));
    assert_eq!(work.state_of("a2", "a"), edge.then_some("connecting"));
    assert_eq!(work.state_of("b1", "b"), Some("active"));
    assert_eq!(
        work.stopped_unasked(
            &[json!({"sessionId":"a1","state":"stopping"})],
            Instant::now(),
        ),
        Vec::<super::super::person_stop::Stopped>::new()
    );
    assert_eq!(
        work.stopped_unasked(
            &[
                json!({"sessionId":"a1","state":"stopped"}),
                json!({"sessionId":"a2","state":"stopping"}),
                json!({"sessionId":"b1","state":"stopped"}),
            ],
            Instant::now(),
        ),
        [
            edge.then(|| super::super::person_stop::Stopped {
                chat: "a".into(),
                browser: Browser::from_profile("edge:p"),
                turn: None,
                connection_ended: false,
            }),
            Some(super::super::person_stop::Stopped {
                chat: "b".into(),
                browser: Browser::from_profile("chrome:p"),
                turn: None,
                connection_ended: false,
            }),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
    );
}

#[test]
fn only_a_connection_loss_reason_drops_the_person_attribution() {
    for (reason, ended) in [
        (json!("ADAPTER_DISCONNECTED"), true),
        (json!("SESSION_CHANGED"), true),
        (json!("LEASE_EXPIRED"), true),
        (json!("STOP_REQUESTED"), false),
        (json!(null), false),
        (json!("SOMETHING_NEW"), false),
    ] {
        let mut work = Work::default();
        work.open("task", "chat", "active", "chrome:p", Some("one"));
        let stopped = work.stopped_unasked(
            &[json!({"sessionId":"task","state":"stopped","suspensionReason":reason})],
            Instant::now(),
        );
        assert_eq!(stopped.len(), 1, "{reason}");
        assert_eq!(stopped[0].connection_ended, ended, "{reason}");
        let words = super::super::person_stop::notice(&stopped[0]);
        assert_eq!(words.ends_with(" by you"), !ended, "{reason}: {words}");
    }
}

/// PREDICT: the helper fences an open the browser never confirmed with `suspensionReason`
/// `START_UNCONFIRMED`; the agent's open already returned `SESSION_NOT_READY`. That reason was unknown here,
/// so the stop kept the person attribution: "Stopped in Your Firefox by you" and the turn was interrupted
/// although nobody touched the browser. Such a fence is not a stop of the agent's work and is not reported.
#[test]
fn a_helper_fence_for_an_unconfirmed_open_is_not_a_person_stop() {
    let mut work = Work::default();
    work.open("task", "chat", "connecting", "firefox:p", Some("one"));
    for state in ["stopping", "stopped"] {
        assert_eq!(
            work.stopped_unasked(
                &[json!({"sessionId":"task","state":state,"suspensionReason":"START_UNCONFIRMED"})],
                Instant::now(),
            ),
            Vec::<super::super::person_stop::Stopped>::new(),
            "{state}"
        );
    }
}

#[test]
fn polls_cannot_undo_an_asked_stop_but_an_explicit_refusal_can() {
    for state in ["active", "connecting"] {
        let mut work = Work::default();
        work.open("task", "chat", state, "chrome:p", None);
        let mut tasks = HashMap::from([("task".into(), "chat".into())]);
        let rows = [json!({"sessionId":"task","state":state})];
        let started = Instant::now();
        work.ask_stop("task", "chat");
        assert!(!work.observe(&rows, &mut tasks, started));
        assert_eq!(work.state_of("task", "chat"), Some("stopping"));
        assert!(!work.observe(&rows, &mut tasks, Instant::now()));
        work.stop_refused("task", "chat");
        assert!(work.observe(&rows, &mut tasks, Instant::now()));
        assert_eq!(work.state_of("task", "chat"), Some(state));
        assert_eq!(tasks.get("task").map(String::as_str), Some("chat"));
        // A later unasked stopping poll still identifies the person's Stop immediately.
        assert_eq!(
            work.stopped_unasked(
                &[json!({"sessionId":"task","state":"stopping"})],
                Instant::now(),
            ),
            vec![super::super::person_stop::Stopped {
                chat: "chat".into(),
                browser: Browser::from_profile("chrome:p"),
                turn: None,
                connection_ended: false,
            }]
        );
    }
}

#[test]
fn a_status_started_before_open_does_not_revoke_the_new_session() {
    let started = Instant::now();
    let mut work = Work::default();
    work.open("new", "chat", "active", "p", None);
    let mut tasks = HashMap::from([("new".into(), "chat".into())]);
    work.observe(&[], &mut tasks, started);
    assert_eq!(tasks.len(), 1);
    assert_eq!(work.view()[0].state, "active");
    work.observe(
        &[json!({"sessionId":"new","state":"active"})],
        &mut tasks,
        Instant::now(),
    );
    assert_eq!(tasks.len(), 1);
}
#[test]
fn stop_releases_authority_only_for_that_chat_and_stays_visible() {
    let mut work = Work::default();
    work.open("a1", "a", "active", "p", None);
    work.open("b1", "b", "active", "p", None);
    let mut tasks = HashMap::from([("a1".into(), "a".into()), ("b1".into(), "b".into())]);
    work.end("a", false);
    assert_eq!(work.view()[0].state, "stopping");
    assert!(work.observe(
        &[
            json!({"sessionId":"a1","state":"stopped"}),
            json!({"sessionId":"b1","state":"active"})
        ],
        &mut tasks,
        Instant::now(),
    ));
    assert_eq!(tasks.len(), 1);
    assert!(tasks.contains_key("b1"));
    assert_eq!(work.view()[0].state, "stopped");
    work.disconnected();
    assert_eq!(work.view()[0].state, "stopped");
    assert_eq!(work.view()[1].state, "disconnected");
    work.end("a", true);
    assert_eq!(work.view().len(), 1);
}
#[test]
fn absent_or_unknown_state_cannot_claim_stopped_and_history_is_bounded() {
    let mut work = Work::default();
    for i in 0..1000 {
        work.open(&format!("s{i}"), &format!("c{i}"), "active", "p", None);
    }
    assert_eq!(work.view().len(), 128);
    work.observe(&[], &mut HashMap::new(), Instant::now());
    assert!(work.view().iter().all(|r| r.state == "disconnected"));
    work.open("replacement", "c999", "page-controlled-text", "p", None);
    assert_eq!(work.view().len(), 127);
    assert_eq!(work.view().last().unwrap().state, "disconnected");
}
