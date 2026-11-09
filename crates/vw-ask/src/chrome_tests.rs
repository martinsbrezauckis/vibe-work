//! Tests for the chat-mode browser authority map.
use super::*;

#[test]
fn choices_are_free_and_absent_from_write_packages() {
    for action in ["page.select", "page.check", "page.uncheck"] {
        assert!(!asks(action, None), "{action}");
        for tier in [Tier::AcceptEdits, Tier::FullAccess] {
            assert!(
                !package_actions(tier).any(|write| write == action),
                "{tier:?} {action}"
            );
        }
    }
    for action in ["page.type", "page.commit", "files.upload"] {
        assert!(asks(action, None), "{action}");
    }
    for action in ["page.reload", "page.back", "page.forward"] {
        assert!(!asks(action, None), "{action}");
    }
}

#[test]
fn chat_modes_have_exact_tiers_and_unknown_modes_have_no_browser() {
    assert_eq!(Tier::from_mode("ask"), Some(Tier::Ask));
    assert_eq!(Tier::from_mode("accept-edits"), Some(Tier::AcceptEdits));
    assert_eq!(Tier::from_mode("smart-approve"), Some(Tier::AcceptEdits));
    assert_eq!(Tier::from_mode("auto-approve"), Some(Tier::FullAccess));
    for mode in ["vw-plan", "plan", "explore", "custom", ""] {
        assert_eq!(Tier::from_mode(mode), None, "{mode}");
    }
}
