//! Ask send authority for the built-in browser: a write card approves a form send only when its words name it.
//! Pure classification; the driver's pre-input send check is covered by vw-browser's real-page tests.
//!
//! PREDICT: before the rule, an approved built-in write card carried send authority whenever the action's Ask tier
//! was above Ask. A "Press Enter" or "Space" card, a typing card and a form fill containing a typing field therefore
//! ran with sending allowed, and the driver skipped its pre-input POST check: Enter in a POST form field or Space on
//! its submit button sent the form under a card that never mentioned sending. Keys also ran outside the hook, so
//! a detected send could not open the send card. Expected red: `may_send` is true for those Ask cards and
//! `activates_send` is false for `press`.
use super::builtin::{activates_send, explain_form_send, may_send};
use serde_json::{Value, json};

fn ask_cards_without_send_words() -> Vec<(&'static str, Value)> {
    vec![
        ("drag", json!({"action":"drag","ref":"e1","to":"e2"})),
        ("enter", json!({"action":"press","key":"Enter","ref":"e1"})),
        ("enter focused", json!({"action":"press","key":"Enter"})),
        ("space", json!({"action":"press","key":"Space","ref":"e1"})),
        ("space char", json!({"action":"press","key":" ","ref":"e1"})),
        ("type", json!({"action":"type","ref":"e1","text":"Hello"})),
        (
            "type newline",
            json!({"action":"type","ref":"e1","text":"Hello\n","submit":false}),
        ),
        (
            "insert",
            json!({"action":"insert_text","ref":"e1","text":"a\nb"}),
        ),
        (
            "form",
            json!({"action":"form","fields":[{"action":"type","ref":"e1","text":"Hi"},{"action":"check","ref":"e2","checked":true}]}),
        ),
        ("click", json!({"action":"click","ref":"e1"})),
        ("page.click", json!({"action":"page.click","x":5,"y":5})),
        ("check", json!({"action":"check","ref":"e1","checked":true})),
    ]
}

#[test]
fn an_ask_write_card_that_does_not_name_a_send_does_not_allow_one() {
    let wrong: Vec<_> = ask_cards_without_send_words()
        .into_iter()
        .filter(|(_, args)| may_send(Some("ask"), args))
        .map(|(name, _)| name)
        .collect();
    assert!(
        wrong.is_empty(),
        "these Ask cards carried send authority: {wrong:?}"
    );
}

#[test]
fn a_card_naming_the_send_and_higher_modes_keep_sending_authority() {
    for args in [
        json!({"action":"type","ref":"e1","text":"Hello","submit":true}),
        json!({"action":"page.dialog.accept","dialogId":"d1"}),
    ] {
        assert!(may_send(Some("ask"), &args), "{args}");
    }
    for mode in [
        Some("accept-edits"),
        Some("smart-approve"),
        Some("full-access"),
        Some("auto-approve"),
        None,
    ] {
        for (name, args) in ask_cards_without_send_words() {
            assert!(may_send(mode, &args), "{mode:?} {name}");
        }
    }
}

#[test]
fn keys_and_activations_run_in_the_hook_so_a_detected_send_asks_with_its_own_card() {
    for action in [
        "click",
        "check",
        "uncheck",
        "press",
        "page.click",
        "page.double_click",
        "page.right_click",
        "page.middle_click",
        "page.triple_click",
    ] {
        assert!(activates_send(&json!({"action":action})), "{action}");
    }
    // Synthetic drag events, text entry and form fills never press a submitter; Ask gives them no send authority.
    for action in [
        "drag",
        "type",
        "insert_text",
        "form",
        "hover",
        "move",
        "select",
    ] {
        assert!(!activates_send(&json!({"action":action})), "{action}");
    }
}

#[test]
fn a_form_fill_stopped_before_a_send_tells_the_agent_to_submit_separately() {
    let mut receipt = json!({"ok":false,"completed":1,"error":vw_browser::SEND_REQUIRED});
    explain_form_send(&mut receipt);
    let text = receipt["error"].as_str().unwrap_or_default();
    assert!(
        text.starts_with("Not run:") && text.contains("separate action"),
        "{text}"
    );
    let mut other = json!({"ok":false,"error":"stale ref"});
    explain_form_send(&mut other);
    assert_eq!(other["error"], "stale ref");
}
