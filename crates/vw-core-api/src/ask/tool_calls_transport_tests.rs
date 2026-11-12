//! Regression tests for `tool_calls.rs`; uses the parent module and is called by Cargo tests.
use super::*;

#[test]
fn tools_with_their_own_policy_never_get_a_generic_card() {
    for name in [
        names::DOCUMENT_READ,
        names::SECRET_RUN,
        names::SECRET_RUN_STATUS,
        names::PROPOSE_PROTECTED_CHANGE,
        names::PROTECTED_CHANGE_STATUS,
        names::BROWSER_CAPTURE_SECRET,
        names::CHROME_TOOL_LIST,
        "chrome_status",
        "chrome_session",
        "chrome_execute",
    ] {
        assert!(asks_inside(name, &json!({})), "{name}");
    }
    for action in ["fill", "close_tab"] {
        assert!(asks_inside(names::BROWSER_ACT, &json!({"action":action})));
    }
    for name in [names::READ_ALOUD, names::USE_HEADLESS_BROWSER] {
        let card =
            json!({"detail":{"kind":"approval","effect":{"toolName":format!("mcp_vw.{name}")}}});
        assert!(
            crate::read_aloud::approve_without_dialog(&card)
                || crate::on_demand::approve_without_dialog(&card),
            "{name}"
        );
    }
    assert!(!asks_inside(
        names::PROCESS_START,
        &json!({"command":"echo hello"})
    ));
    assert!(!asks_inside(names::BROWSER_ACT, &json!({"action":"click"})));
}
