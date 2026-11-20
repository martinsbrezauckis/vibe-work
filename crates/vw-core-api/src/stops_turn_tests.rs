//! A person's Stop belongs to the turn it stopped: a late card of that turn still closes, and a card of any later
//! turn (a scheduled loop's run, a queued message, a turn Vibe starts on its own) is shown once that Stop has ended.
use serde_json::json;

use crate::Command;

fn load(shared: &crate::core::Shared, id: &str, turns: &[(&str, &str)], children: &[&str]) {
    let active = turns
        .iter()
        .rev()
        .find(|(_, s)| *s == "in_progress")
        .map(|(t, _)| *t);
    let state = json!({"session": {"id": id, "harness": "unified", "cwd": "/w",
            "status": match active { Some(t) => json!({"type": "running", "activeTurnId": t}),
                                     None => json!({"type": "idle"}) }},
        "turns": turns.iter().map(|(t, s)| json!({"id": t, "status": s})).collect::<Vec<_>>(),
        "childSessions": children.iter().map(|c| json!({"id": c})).collect::<Vec<_>>(),
        "history": []});
    let _ = shared.store.load(state, vw_session::LoadKind::New);
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

/// The window's Stop of `turn` in chat `session`, counted as `Core::execute` counts it.
fn interrupt(shared: &crate::core::Shared, session: &str, turn: &str) -> Vec<String> {
    super::requested(
        shared,
        &Command::InterruptTurn {
            session_id: session.into(),
            expected_turn_id: turn.into(),
        },
    )
}

/// True when the card closes as a late card.
fn closes(shared: &std::sync::Arc<crate::core::Shared>, turn: Option<&str>) -> bool {
    crate::stop_tree::cancel_late(shared, "s", "s", turn)
}

/// A chat running `t1`, stopped by the person; the Stop has finished (`took`: Vibe took it).
async fn stopped_chat(tag: &str, took: bool) -> crate::Core {
    let core = crate::offline_core::start(tag).await;
    let shared = &core.shared;
    load(shared, "s", &[("t1", "in_progress")], &[]);
    upsert(shared, "t1", "in_progress");
    let stopping = interrupt(shared, "s", "t1");
    super::finished(shared, &stopping, took);
    core
}

/// PREDICT (red before the fix): the person's Stop marked the whole chat, and only the person's own next message
/// lifted the mark, so a scheduled loop's run that came due later had its question cancelled the moment it arrived
/// (`stop_tree::cancel_late`). A Stop now ends only the turn it stopped.
#[tokio::test]
async fn a_loop_run_after_a_finished_stop_shows_its_card() {
    let core = stopped_chat("stops-turn-loop", true).await;
    let shared = &core.shared;
    upsert(shared, "t1", "interrupted");
    // The loop's run: Vibe starts it on its own, no message of the person's.
    upsert(shared, "t2", "in_progress");
    assert!(
        !closes(shared, Some("t2")),
        "the loop run's question stays open"
    );
    assert!(!super::personally_stopped(shared, "s"));
}

/// A card's `callback/call` can reach the core before the pump applies any turn event: the new turn's card itself
/// (its `turnId`) shows that it is not the stopped turn.
#[tokio::test]
async fn a_new_turns_card_before_its_turn_events_is_shown() {
    let core = stopped_chat("stops-turn-card-first", true).await;
    let shared = &core.shared;
    assert!(
        closes(shared, Some("t1")),
        "a late card of the stopped turn"
    );
    assert!(
        closes(shared, None),
        "a card without a turn is not a new turn"
    );
    assert!(
        !closes(shared, Some("t2")),
        "the next turn's card stays open"
    );
    assert!(
        !closes(shared, Some("t1")),
        "the Stop has ended; the chat runs a new turn"
    );
}

/// The safety the Stop keeps: every card of the stopped turn closes, however late, and an event of the stopped turn
/// itself lifts nothing.
#[tokio::test]
async fn late_cards_of_the_stopped_turn_still_close() {
    let core = stopped_chat("stops-turn-late", true).await;
    let shared = &core.shared;
    upsert(shared, "t1", "in_progress");
    for _ in 0..3 {
        assert!(closes(shared, Some("t1")));
    }
    assert!(super::personally_stopped(shared, "s"));
}

/// While the Stop still runs (Vibe has not taken it), no card shows, whatever turn it names. Once Vibe took the
/// interrupt (and its subagents were stopped), a turn other than the stopped one is new: a loop that was due during
/// the stopped turn starts as soon as the chat is idle, and its card shows even before the Stop command returns.
#[tokio::test]
async fn a_stop_holds_until_vibe_took_it() {
    let core = crate::offline_core::start("stops-turn-running").await;
    let shared = &core.shared;
    load(shared, "s", &[("t1", "in_progress")], &[]);
    let stopping = interrupt(shared, "s", "t1");
    upsert(shared, "t2", "in_progress");
    assert!(closes(shared, Some("t2")), "before Vibe took the Stop");
    super::taken(shared, "s");
    assert!(
        closes(shared, Some("t1")),
        "a late card of the stopped turn"
    );
    assert!(!closes(shared, Some("t2")), "the due loop's run");
    shared.asks.open_own(
        "approval",
        Some("s".into()),
        vw_ask::Requester::Agent,
        vec![vw_ask::Choice::Once, vw_ask::Choice::No],
        None,
    );
    super::finished(shared, &stopping, true);
    assert_eq!(
        shared.asks.list().len(),
        1,
        "the end of the Stop leaves the new turn's card open"
    );
}

/// A Stop Vibe refused (a stale turn, a transport failure) holds until the person's next message, as before.
#[tokio::test]
async fn a_refused_stop_holds_until_the_persons_message() {
    let core = stopped_chat("stops-turn-refused", false).await;
    let shared = &core.shared;
    upsert(shared, "t2", "in_progress");
    assert!(closes(shared, Some("t2")));
    upsert(shared, "t1", "interrupted");
    let at_send = super::generation(shared, "s");
    super::sent(
        shared,
        None,
        at_send,
        "session/turn/enqueue",
        &json!({"sessionId": "s", "idempotencyKey": "k", "entries": []}),
        &json!({"queueItemId": "q1"}),
    );
    assert!(!closes(shared, Some("t3")));
}

/// The chat's latest card names the turn a Stop without a turn id (a No, a Stop of the whole chat) ends, even when
/// the pump has not seen that turn start.
#[tokio::test]
async fn a_stop_without_a_turn_id_ends_the_turn_of_the_latest_card() {
    let core = crate::offline_core::start("stops-turn-no").await;
    let shared = &core.shared;
    load(shared, "s", &[], &[]);
    assert!(!closes(shared, Some("t1")), "before any Stop");
    let stopping = super::requested(
        shared,
        &Command::StopSession {
            session_id: "s".into(),
        },
    );
    super::finished(shared, &stopping, true);
    assert!(closes(shared, Some("t1")));
    assert!(!closes(shared, Some("t2")));
}

/// The parent's Stop reaches its subagents: a subagent's card closes until a new turn of the parent lifts the Stop
/// (a subagent card alone never does).
#[tokio::test]
async fn subagent_cards_follow_their_parents_stop() {
    let core = crate::offline_core::start("stops-turn-child").await;
    let shared = &core.shared;
    load(shared, "s", &[("t1", "in_progress")], &["c"]);
    let stopping = interrupt(shared, "s", "t1");
    super::finished(shared, &stopping, true);
    assert!(crate::stop_tree::cancel_late(shared, "c", "s", Some("c9")));
    assert!(super::personally_stopped(shared, "s"));
    upsert(shared, "t2", "in_progress");
    assert!(!crate::stop_tree::cancel_late(shared, "c", "s", Some("c9")));
}

/// A Stop of a subagent stays with it: the parent's next message does not show that subagent's late card; the
/// subagent's own next turn does.
#[tokio::test]
async fn a_subagents_stop_is_not_lifted_by_the_parents_message() {
    let core = crate::offline_core::start("stops-turn-child-own").await;
    let shared = &core.shared;
    load(shared, "s", &[("t1", "completed")], &["c"]);
    let stopping = interrupt(shared, "c", "c1");
    super::finished(shared, &stopping, true);
    let at_send = super::generation(shared, "s");
    super::sent(
        shared,
        None,
        at_send,
        "turn/start",
        &json!({"sessionId": "s", "message": []}),
        &json!({"turn": {"id": "t2"}}),
    );
    assert!(crate::stop_tree::cancel_late(shared, "c", "s", Some("c1")));
    assert!(!crate::stop_tree::cancel_late(shared, "c", "s", Some("c2")));
}

/// "No active turn" skips stopping the subagents (`dispatch_forward`): with subagents the Stop is not taken and holds
/// until the person's next message; without them nothing was running, so it is taken.
#[test]
fn no_active_turn_is_taken_only_without_subagents() {
    let mut error = crate::CoreError::new(
        crate::ErrorCode::ServerError,
        "turn/interrupt: conflict: No active turn",
    );
    error.server_code = Some("conflict".into());
    let result: Result<(), _> = Err(error);
    assert!(super::took(&result, &["s".into()]));
    assert!(!super::took(&result, &["s".into(), "c".into()]));
    let stale: Result<(), _> = Err(crate::CoreError::new(
        crate::ErrorCode::ServerError,
        "stale turn",
    ));
    assert!(!super::took(&stale, &["s".into()]));
}

/// A subagent stopped on its own keeps its Stop when a new turn of its parent lifts the parent's.
#[tokio::test]
async fn a_parents_new_turn_keeps_a_subagents_own_stop() {
    let core = crate::offline_core::start("stops-turn-child-kept").await;
    let shared = &core.shared;
    load(shared, "s", &[("t1", "in_progress")], &["c"]);
    let parent = interrupt(shared, "s", "t1");
    super::finished(shared, &parent, true);
    let child = interrupt(shared, "c", "c1");
    upsert(shared, "t2", "in_progress");
    assert!(!super::personally_stopped(shared, "s"));
    assert!(crate::stop_tree::cancel_late(shared, "c", "s", Some("c1")));
    super::finished(shared, &child, true);
    assert!(crate::stop_tree::cancel_late(shared, "c", "s", Some("c1")));
    assert!(!crate::stop_tree::cancel_late(shared, "c", "s", Some("c2")));
}
