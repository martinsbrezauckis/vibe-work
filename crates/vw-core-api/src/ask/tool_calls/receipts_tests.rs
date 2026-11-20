//! Regression tests for `receipts.rs`; uses the parent module and is called by Cargo tests.
use super::*;

#[test]
fn decision_is_bound_to_owner_source_call_tool_and_input() {
    let calls = Calls::default();
    calls.transports.lock().unwrap().push_back(Receipt {
        key: "decision".into(),
        owner: "parent".into(),
        source: "child".into(),
        call: "call-1".into(),
        tool: "process_start".into(),
        args: json!({"command":"echo once"}),
        prepared_args: Value::Null,
        execution: None,
        stop: 0,
        announced: None,
        issued: std::time::Instant::now(),
    });
    let args = json!({"command":"echo once","_vwReceipt":"decision"});
    assert!(
        verify(
            &calls,
            "parent",
            "child",
            "approval-call-1",
            "process_start",
            &args
        )
        .is_ok()
    );
    assert!(
        verify(
            &calls,
            "parent",
            "child",
            "approval-call-1-1a2b3c4d",
            "process_start",
            &args
        )
        .is_ok()
    );
    for (owner, source, call, tool, input) in [
        (
            "other",
            "child",
            "approval-call-1",
            "process_start",
            args.clone(),
        ),
        (
            "parent",
            "sibling",
            "approval-call-1",
            "process_start",
            args.clone(),
        ),
        (
            "parent",
            "child",
            "approval-call-2",
            "process_start",
            args.clone(),
        ),
        (
            "parent",
            "child",
            "approval-call-1",
            "process_stop",
            args.clone(),
        ),
        (
            "parent",
            "child",
            "approval-call-1",
            "process_start",
            json!({"command":"echo twice","_vwReceipt":"decision"}),
        ),
    ] {
        assert!(verify(&calls, owner, source, call, tool, &input).is_err());
    }
}

/// Seen with Mistral: `history.read {startTime: dayStart, endTime: Date.now()}` was denied "Tool
/// decision does not match this call." The hook's copy and the tool call's copy of the same arguments can spell a
/// whole number beyond 32 bits differently (1793404800000 and 1793404800000.0); they are the same JSON number.
/// PREDICT (before the change): the float spelling of the hook's integer is refused as a mismatch.
#[test]
fn the_same_number_in_either_spelling_matches_and_a_different_number_does_not() {
    let calls = Calls::default();
    let decided: Value = serde_json::from_str(
        r#"{"steps":[{"action":"history.read","args":{"startTime":1793404800000,"limit":100}}]}"#,
    )
    .unwrap();
    calls.transports.lock().unwrap().push_back(Receipt {
        key: "decision".into(),
        owner: "chat".into(),
        source: "chat".into(),
        call: "call-1".into(),
        tool: "chrome_execute".into(),
        args: clean(&decided).unwrap(),
        prepared_args: Value::Null,
        execution: None,
        stop: 0,
        announced: None,
        issued: std::time::Instant::now(),
    });
    let called = |text: &str| -> Value { serde_json::from_str(text).unwrap() };
    let same = called(
        r#"{"steps":[{"action":"history.read","args":{"startTime":1793404800000.0,"limit":100.0}}],"_vwReceipt":"decision"}"#,
    );
    assert!(
        verify(
            &calls,
            "chat",
            "chat",
            "approval-call-1",
            "chrome_execute",
            &same
        )
        .is_ok()
    );
    for other in [
        r#"{"steps":[{"action":"history.read","args":{"startTime":1793404800001.0,"limit":100}}],"_vwReceipt":"decision"}"#,
        r#"{"steps":[{"action":"history.read","args":{"startTime":1793404800000.5,"limit":100}}],"_vwReceipt":"decision"}"#,
        r#"{"steps":[{"action":"history.read","args":{"startTime":"1793404800000","limit":100}}],"_vwReceipt":"decision"}"#,
    ] {
        assert!(
            verify(
                &calls,
                "chat",
                "chat",
                "approval-call-1",
                "chrome_execute",
                &called(other)
            )
            .is_err(),
            "{other}"
        );
    }
}

/// PREDICT (red before the change): the receipt queue was one list of 1000 for every chat, so 1000 receipts of one
/// chat that no Vibe call reports (minted by a holder of the hook credential, or calls Vibe skipped after the hook)
/// made every chat's Vibe Work tools answer "Too many tool calls are in progress", until that chat was stopped. Now
/// the limit counts each chat's own receipts, and an unreported receipt expires after `UNREPORTED_LIFETIME`.
#[tokio::test]
async fn one_chats_unreported_receipts_neither_block_other_chats_nor_stay() {
    let f = crate::feature_test_core::Fixture::new().await;
    let shared = &f.core.shared;
    let receipt = |owner: &str, n: usize| Receipt {
        key: format!("{owner}-{n}"),
        owner: owner.into(),
        source: owner.into(),
        call: format!("call-{n}"),
        tool: "search".into(),
        args: json!({}),
        prepared_args: Value::Null,
        execution: None,
        stop: 0,
        announced: None,
        issued: std::time::Instant::now(),
    };
    for n in 0..1000 {
        save(shared, receipt("busy", n)).unwrap();
    }
    assert_eq!(
        save(shared, receipt("busy", 1000)).err().as_deref(),
        Some("Too many tool calls are in progress.")
    );
    save(shared, receipt("other", 0)).expect("another chat's tools keep working");
    // An unreported receipt expires: once the busy chat's receipts are older than the lifetime, it can call again.
    let long_ago = std::time::Instant::now()
        .checked_sub(UNREPORTED_LIFETIME + Duration::from_secs(1))
        .unwrap();
    for receipt in super::super::calls(shared)
        .unwrap()
        .transports
        .lock()
        .unwrap()
        .iter_mut()
    {
        if receipt.owner == "busy" {
            receipt.issued = long_ago;
        }
    }
    save(shared, receipt("busy", 1001)).expect("expired receipts leave the queue");
    f.finish().await;
}

/// PREDICT: a refused call gave only the fixed text, with no hint which argument changed after the decision; the
/// log now names the differing field paths and never their values.
#[test]
fn a_mismatch_names_the_differing_fields_without_values() {
    let decided = json!({"reason":"r","changes":[{"path":"config.toml","kind":"edit","content":"secret-looking-a"}]});
    let called = json!({"reason":"r","changes":[{"path":"config.toml","kind":"edit","content":"secret-looking-b","note":null}]});
    let mut fields = Vec::new();
    super::differing(&decided, &called, String::new(), &mut fields);
    assert_eq!(fields, ["/changes/0/content", "/changes/0/note"]);
    assert!(fields.iter().all(|f| !f.contains("secret-looking")));
    let mut none = Vec::new();
    super::differing(&decided, &decided, String::new(), &mut none);
    assert_eq!(none, Vec::<String>::new());
}
