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
