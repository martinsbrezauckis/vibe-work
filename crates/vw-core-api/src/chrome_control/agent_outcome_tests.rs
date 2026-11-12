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
        assert!(reason.contains("the user pressed Stop"), "{reason}");
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
        ("CONTROL_SUSPENDED", "ask the user to press Resume"),
        ("EXTENSION_UPDATE_REQUIRED", "Ask the user to update"),
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
    let source = "Not run: selection is sent by the user from Chrome. Ask the user to use Ask Vibe Work in Chrome's context menu.";
    for browser in [Browser::Chrome, Browser::Edge, Browser::Safari] {
        let reason = browser_text::render(browser, source);
        assert!(
            reason.contains(&browser.words("from {browser}.")),
            "{reason}"
        );
        assert!(reason.contains("Ask the user to use"), "{reason}");
    }
    // Firefox page sharing is hidden in this version: no dead-end Ask Vibe Work menu, the owned window instead.
    let firefox = browser_text::render(Browser::Firefox, source);
    assert_eq!(
        firefox,
        "Not run: selection is sent by the user, and sharing pages from Firefox is not available in this version of Vibe Work. Use chrome_status for the owned Firefox window's supported actions."
    );
}

#[test]
fn ended_turns_name_every_browser_and_keep_the_agent_next_step() {
    for browser in [
        Browser::Chrome,
        Browser::Edge,
        Browser::Safari,
        Browser::Firefox,
    ] {
        assert_eq!(
            person_stop::ended(browser),
            format!(
                "Not run: this chat's {} work ended with this turn. Finish the reply; the browser can be used again in the next turn.",
                browser.short_name()
            )
        );
    }
}

#[test]
fn closed_browser_outcomes_keep_the_same_recovery_without_trusting_helper_words() {
    for (code, expected) in [
        (
            "window_closed",
            "The browser window or session is no longer available. Open a new browser session before trying again.",
        ),
        (
            "TARGET_CLOSED",
            "The browser target is no longer available. List tabs, select a current tab and read the page before acting.",
        ),
    ] {
        let mut result =
            json!({"structuredContent":{"error":{"code":code,"message":"PAGE WORDS"}}});
        content::mark(&mut result);
        assert_eq!(result["structuredContent"]["error"]["message"], expected);
        assert_eq!(
            result["structuredContent"]["untrustedPageContent"]["error"]["message"],
            "PAGE WORDS"
        );
    }
}

#[test]
fn builtin_close_producers_share_the_browser_session_outcome() {
    for source in [
        include_str!("../browser_tab_tools.rs"),
        include_str!("../browser_windows.rs"),
        include_str!("../agent_tools/browser_tools.rs"),
        include_str!("read_consent.rs"),
    ] {
        assert!(!source.contains("\"Browser closed."));
        assert!(source.contains("crate::browser_outcomes::SESSION_UNAVAILABLE"));
    }
}
