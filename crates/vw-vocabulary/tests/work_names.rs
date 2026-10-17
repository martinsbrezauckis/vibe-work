//! Checks that core dialogs can use the same project and session names as the UI.
//! Depends on the generated vocabulary lookups; called by Cargo's focused vocabulary tests.

#[test]
fn project_and_session_names_are_shared() {
    for (id, name) in [
        ("projectTab", "Project"),
        ("handbook", "handbook"),
        ("page", "page"),
        ("session", "Session"),
    ] {
        assert_eq!(
            vw_vocabulary::term("workNames", id).map(|term| term.name),
            Some(name)
        );
    }
    assert_eq!(vw_vocabulary::text("newChat"), Some("New chat"));
}
