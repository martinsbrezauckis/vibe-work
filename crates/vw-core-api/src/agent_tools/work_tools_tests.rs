//! The agent's Libraries refusal text when the place the user named is neither a switched-on library nor a
//! knowledge base place.
//!
//! PREDICT: before the refusal named the knowledge base and a next step, it said only "No library is switched on for
//! this chat...", so an agent asked to "look at our library" with no such place kept hunting (project files, the
//! app's folder, connectors) instead of asking the user which place they meant.
use super::none_on;
use crate::knowledge_view::{KnowledgeKind, KnowledgePlace};

fn folder(name: &str) -> KnowledgePlace {
    KnowledgePlace {
        id: format!("id-{name}"),
        name: name.to_owned(),
        kind: KnowledgeKind::Folder,
        location: "/tmp/somewhere".to_owned(),
        label: None,
        note: None,
    }
}

#[test]
fn with_no_place_the_refusal_says_so_and_gives_the_next_step() {
    let text = none_on(&[]);
    assert!(text.contains("No library is switched on"), "{text}");
    assert!(
        text.contains("no places"),
        "says the knowledge base is empty: {text}"
    );
    assert!(
        text.contains("ask the user which place they mean if you cannot find it"),
        "next step: look first, ask if not found: {text}"
    );
    assert!(
        text.contains("Settings > Knowledge base"),
        "where to add it: {text}"
    );
}

#[test]
fn with_other_places_the_refusal_names_them_as_data() {
    let text = none_on(&[folder("price \"archive\""), folder("contracts")]);
    assert!(
        text.contains(r#""price \"archive\"", "contracts""#),
        "the names, quoted: {text}"
    );
    assert!(text.contains("look where it is likely to be"), "{text}");
}

/// Knowledge places are a pointer ("look here first"), never a blocker: the next step must not tell the agent to
/// stop, to ask before looking, or to stay off the computer.
#[test]
fn the_next_step_is_look_first_then_ask_if_not_found() {
    for text in [none_on(&[]), none_on(&[folder("contracts")])] {
        assert!(text.contains("look where it is likely to be"), "{text}");
        assert!(
            text.contains("ask the user which place they mean if you cannot find it"),
            "{text}"
        );
        assert!(text.contains("Settings > Knowledge base"), "{text}");
        assert!(!text.contains("Do not search"), "no search ban: {text}");
        assert!(!text.contains("ask first"), "no ask-first gate: {text}");
    }
}

#[test]
fn a_name_that_matches_nothing_gets_the_same_next_step() {
    let text = super::NOT_FOUND_NEXT;
    assert!(text.contains("look where it is likely to be"), "{text}");
    assert!(
        text.contains("ask the user which place they mean if you cannot find it"),
        "{text}"
    );
    assert!(!text.contains("Do not search"), "{text}");
    assert!(!text.contains("ask first"), "{text}");
}
