//! Checks that core dialogs can use the same project and session names as the UI.
//! Depends on the generated vocabulary lookups; called by Cargo's focused vocabulary tests.

#[test]
fn project_and_session_names_are_shared() {
    for (id, name) in [
        ("projectTab", "Project"),
        ("handbook", "handbook"),
        ("page", "page"),
        ("session", "Chat"),
    ] {
        assert_eq!(
            vw_vocabulary::term("workNames", id).map(|term| term.name),
            Some(name)
        );
    }
    assert_eq!(vw_vocabulary::text("newChat"), Some("New chat"));
}

#[test]
fn identical_copy_keeps_both_lookup_keys() {
    for id in ["accessDefaultDescription", "defaultThinkingDescription"] {
        assert_eq!(vw_vocabulary::text(id), Some("Applies to new chats."));
    }
}
