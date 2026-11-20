//! Stop generation rules on a core without Vibe: a Stop moves the generation before it waits for any lock, a turn
//! that ends failed or interrupted invalidates old decisions; Stop settles cards after runtime cancellation.
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Waker},
};

/// Poll once while the test holds the lock, then retain the queued future until the lock is released.
fn queued(future: Pin<&mut impl Future>) {
    assert!(
        future
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
}

use serde_json::json;
use vw_ask::{Choice, Outcome};

use crate::{Command, Origin};

/// A continuation that queued on the chat's send lock before the person's Stop must not be sent: the Stop moves
/// the generation first and only then waits for the same lock.
#[tokio::test]
async fn stop_session_moves_the_generation_before_it_waits_for_the_chat_lock() {
    let core = crate::offline_core::start("stops-before-lock").await;
    let scheduled = super::generation(&core.shared, "s");
    let held = core.shared.plugin_refresh.lock("s").await;
    let continuation = core.forward_unless_stopped(
        "turn/start",
        json!({"sessionId":"s","message":[{"type":"text","text":"continue"}],"injected":true}),
        scheduled,
    );
    tokio::pin!(continuation);
    // Polling while the lock is held puts this continuation first in Tokio's fair mutex queue.
    queued(continuation.as_mut());
    let stop = core.execute(
        Origin::Ui,
        Command::StopSession {
            session_id: "s".into(),
        },
    );
    tokio::pin!(stop);
    let stop_poll = stop.as_mut().poll(&mut Context::from_waker(Waker::noop()));
    assert_ne!(super::generation(&core.shared, "s"), scheduled);
    drop(held);
    let sent = continuation.await;
    if stop_poll.is_pending() {
        let _ = stop.await;
    }
    let error = sent.expect_err("nothing may be sent after the Stop");
    assert!(
        error.code == crate::ErrorCode::Denied && error.message.contains("stopped"),
        "a continuation scheduled before the Stop is refused as stopped, not sent: {error:?}"
    );
}

/// A parent turn event must not release hooks still owned by its children.
#[tokio::test]
async fn a_failed_turn_invalidates_decisions_without_releasing_held_hooks() {
    let core = crate::offline_core::start("stops-failed-turn").await;
    let shared = &core.shared;
    let handle = shared.asks.open_own(
        "approval",
        Some("s".into()),
        vw_ask::Requester::Agent,
        vec![Choice::Once, Choice::No],
        None,
    );
    super::observe(
        shared,
        &vw_session::Change::TurnUpserted {
            session_id: "s".into(),
            turn: json!({"id":"t1","status":"failed"}),
        },
    );
    assert_eq!(super::generation(shared, "s"), 1);
    assert!(
        shared.asks.get(&handle.ask.id).is_some(),
        "the hook still owns its prompt"
    );
    let stopping = super::requested(
        shared,
        &Command::StopSession {
            session_id: "s".into(),
        },
    );
    assert!(
        shared.asks.get(&handle.ask.id).is_some(),
        "Stop must reach Vibe before settling prompts"
    );
    super::finished(shared, &stopping, true);
    assert!(matches!(
        shared.asks.outcome(&handle.ask.id).map(|r| r.outcome),
        Some(Outcome::Cancelled { .. })
    ));
}

/// The tool card's generation is captured when the agent's call is checked; a Stop that lands before the card
/// registers its ask cancels nothing (no ask yet). The card must then close at once instead of staying open on a
/// stopped chat with an Allow that would start a turn.
#[tokio::test]
async fn a_tool_card_that_opens_after_a_stop_closes_at_once() {
    let core = crate::offline_core::start("stops-late-card").await;
    let shared = &core.shared;
    let checked = super::generation(shared, "s");
    super::stopped(shared, "s");
    crate::ask::callback_opened_at(
        shared,
        &json!({"id":"vw-tool-2","sessionId":"s","detail":{"kind":"approval","effect":{"toolName":"mcp_vw.process_start","input":{}},"requiredPermissions":[]}}),
        "approval",
        "s",
        Some(checked),
    );
    assert!(
        shared.asks.get("callback:vw-tool-2").is_none(),
        "no card stays open on a chat stopped after its call was checked: {:?}",
        shared.asks.list()
    );
}

/// A checked app send started turn `t2`; the window's Stop names `t1`, the turn it still shows. Under the chat's
/// send lock the interrupt is pointed at `t2`, once. A turn the person started, or the end of `t2`, forgets it.
#[tokio::test]
async fn a_stop_after_a_checked_start_names_the_turn_that_start_began() {
    let core = crate::offline_core::start("stops-retarget").await;
    let shared = &core.shared;
    let stop = |turn: &str| json!({"sessionId": "s", "expectedTurnId": turn});
    let checked = super::generation(shared, "s");
    super::sent(
        shared,
        Some(checked),
        0,
        "turn/start",
        &json!({"sessionId": "s"}),
        &json!({"turn": {"id": "t2"}}),
    );
    // No Stop since the check: an interrupt is left as the window sent it.
    assert_eq!(
        super::retarget(shared, "turn/interrupt", stop("t1")),
        stop("t1")
    );
    super::stopped(shared, "s");
    assert_eq!(
        super::retarget(shared, "turn/interrupt", stop("t1")),
        stop("t2")
    );
    assert_eq!(
        super::retarget(shared, "turn/interrupt", stop("t1")),
        stop("t1"),
        "one Stop takes it"
    );
    // Other calls are never touched.
    assert_eq!(
        super::retarget(shared, "turn/steer", stop("t1")),
        stop("t1")
    );

    // The person's own start: the chat runs their turn now.
    super::sent(
        shared,
        Some(super::generation(shared, "s")),
        0,
        "turn/start",
        &json!({"sessionId": "s"}),
        &json!({"turn": {"id": "t3"}}),
    );
    super::sent(
        shared,
        None,
        0,
        "turn/start",
        &json!({"sessionId": "s"}),
        &json!({"turn": {"id": "t4"}}),
    );
    super::stopped(shared, "s");
    assert_eq!(
        super::retarget(shared, "turn/interrupt", stop("t4")),
        stop("t4")
    );

    // The started turn ended: nothing is left to point at.
    super::sent(
        shared,
        Some(super::generation(shared, "s")),
        0,
        "turn/start",
        &json!({"sessionId": "s"}),
        &json!({"turn": {"id": "t5"}}),
    );
    super::observe(
        shared,
        &vw_session::Change::TurnUpserted {
            session_id: "s".into(),
            turn: json!({"id": "t5", "status": "completed"}),
        },
    );
    super::stopped(shared, "s");
    assert_eq!(
        super::retarget(shared, "turn/interrupt", stop("t6")),
        stop("t6")
    );
}

/// A minimal unified session state for chat `id` whose turns are `turns` (id, status); the last in-progress one is
/// the active turn.
fn chat(id: &str, turns: &[(&str, &str)]) -> serde_json::Value {
    let active = turns
        .iter()
        .rev()
        .find(|(_, s)| *s == "in_progress")
        .map(|(t, _)| *t);
    json!({"session": {"id": id, "harness": "unified", "cwd": "/w",
            "status": match active { Some(t) => json!({"type": "running", "activeTurnId": t}),
                                     None => json!({"type": "idle"}) }},
        "turns": turns.iter().map(|(t, s)| json!({"id": t, "status": s})).collect::<Vec<_>>(),
        "history": []})
}

/// The pump saw the continuation's quick turn `t2` end before the start's caller recorded it; then the person's
/// next turn `t3` started (a queued or voice message, which records nothing). A Stop naming `t3` stays on `t3`.
#[tokio::test]
async fn a_started_turn_that_already_ended_is_never_the_stop_target() {
    let core = crate::offline_core::start("stops-ended-record").await;
    let shared = &core.shared;
    let _ = shared
        .store
        .load(chat("s", &[("t2", "completed")]), vw_session::LoadKind::New);
    super::observe(
        shared,
        &vw_session::Change::TurnUpserted {
            session_id: "s".into(),
            turn: json!({"id": "t2", "status": "completed"}),
        },
    );
    super::sent(
        shared,
        Some(super::generation(shared, "s")),
        0,
        "turn/start",
        &json!({"sessionId": "s"}),
        &json!({"turn": {"id": "t2"}}),
    );
    let _ = shared.store.load(
        chat("s", &[("t2", "completed"), ("t3", "in_progress")]),
        vw_session::LoadKind::New,
    );
    super::stopped(shared, "s");
    let stop = json!({"sessionId": "s", "expectedTurnId": "t3"});
    assert_eq!(
        super::retarget(shared, "turn/interrupt", stop.clone()),
        stop
    );
}

/// A conversation Vibe imported from id `a` as `b`, stopped by the old id: the Stop reaches the live chat `b` (its
/// snapshot, channel and open asks), not only its lock.
#[tokio::test]
async fn a_stop_by_an_imported_id_cancels_the_current_ids_cards() {
    let core = crate::offline_core::start("stops-imported").await;
    let shared = &core.shared;
    shared.channels.imported_as("a", "b");
    let _ = shared
        .store
        .load(chat("b", &[("t1", "completed")]), vw_session::LoadKind::New);
    let card = shared.asks.open_own(
        "approval",
        Some("b".into()),
        vw_ask::Requester::Agent,
        vec![Choice::Once, Choice::No],
        None,
    );
    // This offline fixture checks imported identity on an idle chat. Active runtime Stop uses real Vibe tests.
    // As `Core::execute` runs it, past the session lease (which needs a Vibe server).
    let stop = Command::StopSession {
        session_id: "a".into(),
    };
    let stopping = super::requested(shared, &stop);
    let stopped = core.local(Origin::Ui, stop).await;
    super::finished(shared, &stopping, true);
    assert!(stopped.is_ok(), "{stopped:?}");
    assert!(
        shared.asks.get(&card.ask.id).is_none(),
        "the live chat's open card was cancelled"
    );
}

/// Chats that never saw a Stop are forgotten when they are removed; a stopped chat keeps its generation (an old
/// continuation must not see generation 0 again).
#[tokio::test]
async fn a_removed_chat_without_a_stop_is_forgotten() {
    let core = crate::offline_core::start("stops-prune").await;
    let shared = &core.shared;
    for id in ["x", "y"] {
        super::sent(
            shared,
            Some(0),
            0,
            "turn/start",
            &json!({"sessionId": id}),
            &json!({"turn": {"id": "t1"}}),
        );
    }
    super::stopped(shared, "y");
    for id in ["x", "y"] {
        super::observe(
            shared,
            &vw_session::Change::Removed {
                session_id: id.into(),
            },
        );
    }
    assert!(!shared.stops.chats().contains_key("x"), "x is forgotten");
    assert_eq!(super::generation(shared, "y"), 1, "y keeps its generation");
}

/// Secret-run completion metadata steers the running turn; it obeys Stop like every app send. Queued behind the chat
/// lock before the Stop, it is refused after it.
#[tokio::test]
async fn a_secret_completion_queued_before_a_stop_is_not_sent() {
    let core = crate::offline_core::start("stops-secret-queued").await;
    let shared = core.shared.clone();
    let _ = shared.store.load(
        chat("s", &[("t1", "in_progress")]),
        vw_session::LoadKind::New,
    );
    let held = shared.plugin_refresh.lock("s").await;
    let finished =
        crate::ask::continuation::secret_finished(shared.clone(), "s".into(), "secret-1".into());
    tokio::pin!(finished);
    queued(finished.as_mut());
    super::stopped(&shared, "s");
    drop(held);
    let sent = finished.await.expect("a running turn is woken");
    let error = sent.expect_err("nothing may be sent after the Stop");
    assert!(super::refused_as_stopped(&error), "{error:?}");
}

/// The person pressed Stop while the turn ran and Vibe refused it (the window named another turn): a secret-run
/// completion that arrives afterwards does not steer that turn.
#[tokio::test]
async fn a_secret_completion_after_an_accepted_stop_does_not_steer_the_turn() {
    let core = crate::offline_core::start("stops-secret-after").await;
    let shared = core.shared.clone();
    let _ = shared.store.load(
        chat("s", &[("t1", "in_progress")]),
        vw_session::LoadKind::New,
    );
    super::observe(
        &shared,
        &vw_session::Change::TurnUpserted {
            session_id: "s".into(),
            turn: json!({"id": "t1", "status": "in_progress"}),
        },
    );
    super::stopped(&shared, "s");
    let sent =
        crate::ask::continuation::secret_finished(shared.clone(), "s".into(), "secret-2".into())
            .await
            .expect("a running turn is woken");
    let error = sent.expect_err("nothing may be sent after the Stop");
    assert!(super::refused_as_stopped(&error), "{error:?}");
}

/// A Stop walks ownership to grandchildren, while a Branch's parentSessionId grants no ownership.
#[tokio::test]
async fn stop_marks_descendants_but_leaves_branches_independent() {
    let core = crate::offline_core::start("stops-family").await;
    let shared = &core.shared;
    for (id, children, parent) in [
        ("root", vec!["child"], None),
        ("child", vec!["grandchild"], Some("root")),
        ("grandchild", vec![], Some("child")),
        ("branch", vec![], Some("root")),
    ] {
        let mut state = chat(id, &[]);
        state["childSessions"] = json!(
            children
                .iter()
                .map(|id| json!({"id":id}))
                .collect::<Vec<_>>()
        );
        state["session"]["parentSessionId"] = json!(parent);
        let _ = shared.store.load(state, vw_session::LoadKind::New);
        shared.asks.open_own(
            "approval",
            Some(id.into()),
            vw_ask::Requester::Agent,
            vec![Choice::Once, Choice::No],
            None,
        );
    }
    let stopping = super::requested(
        shared,
        &Command::StopSession {
            session_id: "root".into(),
        },
    );
    for id in ["root", "child", "grandchild"] {
        assert!(super::personally_stopped(shared, id));
        assert_eq!(super::generation(shared, id), 1);
    }
    assert_eq!(super::generation(shared, "branch"), 0);
    assert_eq!(
        shared.asks.list().len(),
        4,
        "no held hook released before runtime Stop"
    );
    super::finished(shared, &stopping, true);
    assert_eq!(shared.asks.list().len(), 1);
    assert_eq!(shared.asks.list()[0].session.as_deref(), Some("branch"));
}

/// An idle root must keep the runtime carrying its busy child, including when only the summary is loaded.
#[tokio::test]
async fn idle_parent_is_not_quiet_while_owned_children_work() {
    let core = crate::offline_core::start("stops-quiet-tree").await;
    let shared = &core.shared;
    let mut state = chat("parent", &[("done", "completed")]);
    state["session"]["status"] = json!({"type":"idle"});
    state["childSessions"] = json!([{"id":"child","status":{"type":"running"}}]);
    let _ = shared.store.load(state.clone(), vw_session::LoadKind::New);
    assert!(!crate::recovery::quiet(shared, "parent"));
    state["childSessions"][0]["status"] = json!({"type":"idle"});
    let _ = shared.store.load(state, vw_session::LoadKind::New);
    assert!(crate::recovery::quiet(shared, "parent"));
    let ask = shared.asks.open_own(
        "approval",
        Some("child".into()),
        vw_ask::Requester::Agent,
        vec![Choice::Once, Choice::No],
        None,
    );
    assert!(
        !crate::recovery::quiet(shared, "parent"),
        "a waiting card can outlive the child's status"
    );
    shared
        .asks
        .cancel_session(Some("child"), "the turn was stopped");
    assert!(shared.asks.outcome(&ask.ask.id).is_some());
    assert!(crate::recovery::quiet(shared, "parent"));
    let mut branch = chat("branch", &[("busy", "in_progress")]);
    branch["session"]["parentSessionId"] = json!("parent");
    branch["session"]["status"] = json!({"type":"running","activeTurnId":"busy"});
    let _ = shared.store.load(branch, vw_session::LoadKind::New);
    assert!(
        crate::recovery::quiet(shared, "parent"),
        "a Branch does not keep its source runtime busy"
    );
}

#[test]
fn stopped_refusal_is_recognized_by_code_after_wording_changes() {
    let mut stopped = crate::CoreError::new(crate::ErrorCode::Denied, "The continuation ended.");
    stopped.server_code = Some("stopped".to_owned());
    assert!(super::refused_as_stopped(&stopped));
    let unrelated = crate::CoreError::new(
        crate::ErrorCode::Denied,
        "Not sent: the chat was stopped after this continuation was scheduled.",
    );
    assert!(!super::refused_as_stopped(&unrelated));
}

fn upsert(shared: &crate::core::Shared, turn: &str, status: &str) {
    super::observe(
        shared,
        &vw_session::Change::TurnUpserted {
            session_id: "s".into(),
            turn: json!({"id": turn, "status": status}),
        },
    );
}

/// The person's queued message (`session/turn/enqueue`, as voice chat sends every message) as Vibe accepted it.
fn queued_by_person(shared: &crate::core::Shared) {
    let at_send = super::generation(shared, "s");
    super::sent(
        shared,
        None,
        at_send,
        "session/turn/enqueue",
        &json!({"sessionId": "s", "idempotencyKey": "k", "entries": []}),
        &json!({"queueItemId": "q1"}),
    );
}

/// PREDICT (red before the fix): a person's Stop holds until their next message. Voice chat (and a message typed
/// while voice chat is on) is queued with `session/turn/enqueue`; only the person's own `turn/start` lifted the Stop,
/// so after one Stop every later question or approval card of the chat was cancelled the moment it arrived
/// (`stop_tree::cancel_late`) and the person never saw it. The queued message lifts it when Vibe accepts it, once
/// the stopped turn has ended; while that turn still ends, a late card of it still closes.
#[tokio::test]
async fn the_persons_queued_message_lifts_a_stop_once_the_stopped_turn_ended() {
    let core = crate::offline_core::start("stops-queued-lifts").await;
    let shared = &core.shared;
    let _ = shared.store.load(
        chat("s", &[("t1", "in_progress")]),
        vw_session::LoadKind::New,
    );
    upsert(shared, "t1", "in_progress");
    super::stopped(shared, "s");
    assert!(
        crate::stop_tree::cancel_late(shared, "s", "s", None),
        "right after the Stop a late card closes"
    );
    // Queued while the stopped turn still ends: a late card of that turn still closes.
    queued_by_person(shared);
    assert!(
        crate::stop_tree::cancel_late(shared, "s", "s", None),
        "a late card of the stopped turn still closes"
    );
    upsert(shared, "t1", "interrupted");
    queued_by_person(shared);
    assert!(
        !super::personally_stopped(shared, "s"),
        "the person's next message lifts their Stop"
    );
    assert!(
        !crate::stop_tree::cancel_late(shared, "s", "s", None),
        "a question of the person's next turn stays open, even before its turn shows"
    );
}

/// The stopped turn's end event never reached the core (a lagged subscription, a closed channel), but the chat no
/// longer runs it: the person's queued message still lifts the Stop. While the Stop has not finished, a later turn
/// (`stops_turn_tests.rs` covers a finished one) and an app continuation do not lift it.
#[tokio::test]
async fn a_queued_message_lifts_a_stop_without_the_end_event_and_nothing_else_does() {
    let core = crate::offline_core::start("stops-queued-missed-end").await;
    let shared = &core.shared;
    let _ = shared.store.load(
        chat("s", &[("t1", "in_progress")]),
        vw_session::LoadKind::New,
    );
    upsert(shared, "t1", "in_progress");
    super::stopped(shared, "s");
    let _ = shared.store.load(
        chat("s", &[("t1", "interrupted")]),
        vw_session::LoadKind::Resume,
    );
    upsert(shared, "t2", "in_progress");
    assert!(
        super::personally_stopped(shared, "s"),
        "a turn during an unfinished Stop does not lift it"
    );
    let checked = super::generation(shared, "s");
    super::sent(
        shared,
        Some(checked),
        checked,
        "session/turn/enqueue",
        &json!({"sessionId": "s", "idempotencyKey": "k", "entries": []}),
        &json!({"queueItemId": "q1"}),
    );
    assert!(
        super::personally_stopped(shared, "s"),
        "an app continuation does not lift it"
    );
    queued_by_person(shared);
    assert!(!super::personally_stopped(shared, "s"));
}

/// PREDICT (red before the fix): a Stop the person pressed while their message was on its way (counted after the
/// send was read, `at_send`) was lifted by that same message once Vibe accepted it, so the cards of the turn the Stop
/// was meant for were shown. A message lifts only a Stop that came before it was sent.
#[tokio::test]
async fn a_stop_pressed_while_the_message_is_on_its_way_holds() {
    let core = crate::offline_core::start("stops-during-send").await;
    let shared = &core.shared;
    let _ = shared.store.load(chat("s", &[]), vw_session::LoadKind::New);
    for method in ["session/turn/enqueue", "turn/start"] {
        let at_send = super::generation(shared, "s");
        super::stopped(shared, "s");
        super::sent(
            shared,
            None,
            at_send,
            method,
            &json!({"sessionId": "s", "idempotencyKey": "k", "entries": []}),
            &json!({}),
        );
        assert!(
            super::personally_stopped(shared, "s"),
            "{method}: the Stop pressed during the send holds"
        );
    }
}

/// The stopped turn's end, seen while the person's message is on its way, raises the chat's generation; it is not a
/// new Stop, so the message still lifts the Stop it followed.
#[tokio::test]
async fn the_stopped_turns_end_seen_during_the_send_does_not_keep_the_stop() {
    let core = crate::offline_core::start("stops-end-during-send").await;
    let shared = &core.shared;
    let _ = shared.store.load(chat("s", &[]), vw_session::LoadKind::New);
    upsert(shared, "t1", "in_progress");
    super::stopped(shared, "s");
    for method in ["session/turn/enqueue", "turn/start"] {
        let at_send = super::generation(shared, "s");
        upsert(shared, "t1", "interrupted");
        super::sent(
            shared,
            None,
            at_send,
            method,
            &json!({"sessionId": "s", "idempotencyKey": "k", "entries": []}),
            &json!({}),
        );
        assert!(
            !super::personally_stopped(shared, "s"),
            "{method}: the person's message lifts the Stop"
        );
        super::stopped(shared, "s");
    }
}
