//! Trusted browser outcomes identify the person and give the agent an actionable next step.
//! Exercises fixed app diagnostics without an extension, native view, or page text.
use super::{agent_browser::Browser, browser_text, content, person_stop};
use serde_json::json;

#[test]
fn stop_outcomes_name_the_person_and_keep_the_same_next_step() {
    for browser in [
        Browser::Chrome,
        Browser::Edge,
        Browser::Firefox,
        Browser::Safari,
    ] {
        let reason = person_stop::refusal(browser);
        assert!(reason.contains("the person pressed Stop"), "{reason}");
        assert!(
            reason.contains("tell them what you were doing and wait for their answer"),
            "{reason}"
        );
        assert_eq!(
            reason,
            vw_browser::STOPPED.replace("Vibe Work's browser", browser.name())
        );
    }
}

#[test]
fn paused_control_and_extension_updates_ask_the_person_to_act() {
    for (code, next) in [
        ("CONTROL_SUSPENDED", "ask the person to press Resume"),
        ("EXTENSION_UPDATE_REQUIRED", "Ask the person to update"),
    ] {
        let mut result =
            json!({"structuredContent":{"error":{"code":code,"message":"PAGE WORDS"}}});
        content::mark(&mut result);
        let reason = result["structuredContent"]["error"]["message"]
            .as_str()
            .unwrap();
        assert!(reason.contains(next), "{reason}");
        assert!(!reason.contains("PAGE WORDS"));
    }
}

#[test]
fn selection_instructions_keep_the_person_as_the_actor_in_each_browser() {
    let source = "Not run: selection is sent by the person from Chrome. Ask the person to use Ask Vibe Work in Chrome's context menu.";
    for browser in [
        Browser::Chrome,
        Browser::Edge,
        Browser::Firefox,
        Browser::Safari,
    ] {
        let reason = browser_text::render(browser, source);
        assert!(
            reason.contains(&browser.words("from {browser}.")),
            "{reason}"
        );
        assert!(reason.contains("Ask the person to use"), "{reason}");
    }
}
