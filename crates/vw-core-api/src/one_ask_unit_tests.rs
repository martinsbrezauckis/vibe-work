//! Regression tests for `one_ask.rs`; uses the parent module and is called by Cargo tests.
use super::{Ledger, approve_once};
use crate::command::Origin;
use serde_json::{Value, json};

fn card(session: &str, id: &str, tool: &str, input: &Value) -> Value {
    json!({"id": id, "callbackId": id, "sessionId": session, "type": "callback",
            "detail": {"kind": "approval", "effect": {"toolName": tool, "kind": "shell", "input": input}}})
}

#[test]
fn an_answered_card_is_reported_to_its_tool_once_and_revoked_on_send_failure() {
    let ledger = Ledger::default();
    for id in ["approval-one-1234abcd", "approval-two-1234abcd"] {
        ledger.card_opened(&card(
            "s",
            id,
            "file_system.read_file",
            &json!({"file_path":"reference.md"}),
        ));
        ledger.card_answered("s", id, &approve_once(), Origin::Ui);
    }
    assert!(!ledger.user_allowed("other", "one"));
    assert!(!ledger.user_allowed("s", "one-more"));
    assert!(ledger.user_allowed("s", "one"));
    assert!(!ledger.user_allowed("s", "one"));
    ledger.card_answer_failed("s", "approval-two-1234abcd");
    assert!(!ledger.user_allowed("s", "two"));
}

#[test]
fn a_guard_yes_answers_exactly_its_own_card_once() {
    let ledger = Ledger::default();
    let cmd = json!({"command": "git fetch"});
    // Nothing remembered: no card is answered for the user.
    assert!(!ledger.answer_for_card(&card("s", "approval-c1", "file_system.bash", &cmd)));
    ledger.guard_said_yes(&["s", "parent"], "c1", "git fetch");
    // Another call id, another session, another command, not an approval: no.
    assert!(!ledger.answer_for_card(&card("s", "approval-c2", "file_system.bash", &cmd)));
    assert!(!ledger.answer_for_card(&card("x", "approval-c1", "file_system.bash", &cmd)));
    assert!(!ledger.answer_for_card(&card(
        "s",
        "approval-c1",
        "file_system.bash",
        &json!({"command": "git fetch --all"})
    )));
    let mut question = card("s", "approval-c1", "file_system.bash", &cmd);
    question["detail"]["kind"] = json!("user_input");
    assert!(!ledger.answer_for_card(&question));
    // The parent session's card for the same call: yes, once.
    assert!(ledger.answer_for_card(&card("parent", "approval-c1", "file_system.bash", &cmd)));
    assert!(!ledger.answer_for_card(&card("parent", "approval-c1", "file_system.bash", &cmd)));
    // An empty call id (an old hook) is never remembered.
    ledger.guard_said_yes(&["s"], "", "git fetch");
    assert!(!ledger.answer_for_card(&card("s", "approval-", "file_system.bash", &cmd)));
    assert_eq!(approve_once()["decision"]["type"], "approve");
}

#[test]
fn callback_identity_accepts_only_the_known_call_and_exact_nonce_shape() {
    use super::callback_matches_call;
    assert!(callback_matches_call("approval-call-1", "call-1"));
    assert!(callback_matches_call("approval-call-1-1a2b3c4d", "call-1"));
    for callback in [
        "approval-call-2-1a2b3c4d",
        "approval-call-1-1234567",
        "approval-call-1-123456789",
        "approval-call-1-1A2B3C4D",
        "approval-call-1-nothex!!",
        "approval-call-1-extra-1a2b3c4d",
        "question-call-1-1a2b3c4d",
    ] {
        assert!(!callback_matches_call(callback, "call-1"), "{callback}");
    }
    assert!(!callback_matches_call("approval--1a2b3c4d", ""));
}

#[test]
fn a_unique_callback_nonce_keeps_the_exact_guard_decision() {
    let ledger = Ledger::default();
    let cmd = json!({"command":"git fetch"});
    ledger.guard_said_yes(&["s"], "call-1", "git fetch");
    assert!(!ledger.answer_for_card(&card(
        "s",
        "approval-call-2-1a2b3c4d",
        "file_system.bash",
        &cmd
    )));
    assert!(!ledger.answer_for_card(&card(
        "s",
        "approval-call-1-extra-1a2b3c4d",
        "file_system.bash",
        &cmd
    )));
    assert!(ledger.answer_for_card(&card(
        "s",
        "approval-call-1-1a2b3c4d",
        "file_system.bash",
        &cmd
    )));
    assert!(!ledger.answer_for_card(&card(
        "s",
        "approval-call-1-9a2b3c4d",
        "file_system.bash",
        &cmd
    )));
}

#[test]
fn a_failed_card_send_revokes_only_that_approval() {
    let ledger = Ledger::default();
    let args = json!({"command":"git rebase main"});
    let tool = "mcp_vw.process_start";
    for id in ["approval-a", "approval-b"] {
        ledger.card_opened(&card("s", id, tool, &args));
        ledger.card_answered("s", id, &approve_once(), Origin::Ui);
    }
    ledger.card_answer_failed("s", "approval-a");
    assert!(ledger.card_said_yes("s", tool, &args));
    assert!(!ledger.card_said_yes("s", tool, &args));
}

#[test]
fn a_card_yes_counts_for_the_same_tool_call_only() {
    let ledger = Ledger::default();
    let args = json!({"command": "git fetch && npm run dev"});
    let tool = "mcp_vw.process_start";
    ledger.card_opened(&card("s", "approval-t1", tool, &args));
    ledger.card_opened(&card("s", "approval-t2", tool, &args));
    // Denied: nothing remembered.
    ledger.card_answered(
        "s",
        "approval-t1",
        &json!({"type": "approval", "decision": {"type": "deny"}}),
        Origin::Ui,
    );
    assert!(!ledger.card_said_yes("s", tool, &args));
    // An answer for an unknown card: nothing.
    ledger.card_answered("s", "approval-zz", &approve_once(), Origin::Ui);
    assert!(!ledger.card_said_yes("s", tool, &args));
    ledger.card_answered(
        "s",
        "approval-t2",
        &json!({"type": "approval", "decision": {"type": "approve_for_session"}}),
        Origin::DebugApi,
    );
    assert!(!ledger.card_said_yes("other", tool, &args));
    assert!(!ledger.card_said_yes("s", "mcp_vw.secret_run", &args));
    assert!(!ledger.card_said_yes("s", tool, &json!({"command": "git fetch"})));
    assert!(ledger.card_said_yes("s", tool, &args));
    assert!(
        !ledger.card_said_yes("s", tool, &args),
        "used once: the next call asks again"
    );
}
