//! Tests for the chat-mode browser authority map.
use super::*;

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
