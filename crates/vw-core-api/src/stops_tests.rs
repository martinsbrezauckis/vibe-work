//! Stop generation rules on a core without Vibe: a Stop moves the generation before it waits for any lock, a turn
//! that ends failed or interrupted invalidates old decisions; Stop settles cards after runtime cancellation.
use std::time::Duration;

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
    let continuation = {
        let core = core.clone();
        tokio::spawn(async move {
            core.forward_unless_stopped(
                "turn/start",
                json!({"sessionId":"s","message":[{"type":"text","text":"continue"}],"injected":true}),
                scheduled,
            )
            .await
        })
    };
    // The continuation is first in the lock's queue (tokio's mutex is fair), then the Stop arrives.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let stop = {
        let core = core.clone();
        tokio::spawn(async move {
            core.execute(
                Origin::Ui,
                Command::StopSession {
                    session_id: "s".into(),
                },
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(held);
    let sent = continuation.await.unwrap();
    let _ = stop.await.unwrap();
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
        vw_ask::chrome::OTHER_KIND,
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
    super::finished(shared, &stopping);
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
        vw_ask::chrome::OTHER_KIND,
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
    super::finished(shared, &stopping);
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
    let finished = tokio::spawn(crate::ask::continuation::secret_finished(
        shared.clone(),
        "s".into(),
        "secret-1".into(),
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;
    super::stopped(&shared, "s");
    drop(held);
    let sent = finished.await.unwrap().expect("a running turn is woken");
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
            vw_ask::chrome::OTHER_KIND,
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
    super::finished(shared, &stopping);
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
        vw_ask::chrome::OTHER_KIND,
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
