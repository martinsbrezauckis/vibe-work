//! The tool result of a call the user said No to: the agent is told that nothing happened.
//!
//! PREDICT: the decline text said what the agent should do next (explain why the task needs it) but not that nothing
//! happened. A model whose script ran other steps reported the declined click as done ("Clicked Sign in") while the
//! gate had held. These assertions fail on that text; the window still recognizes the text by its first sentence.

#[test]
fn a_decline_says_nothing_was_done_and_must_not_be_reported_as_done() {
    assert_eq!(
        vw_ask::DECLINED,
        "Not run: the user declined this. Nothing was done, so report it as not done, never as done. Explain why it is needed for the user's delegated task."
    );
    assert!(vw_ask::DECLINED.starts_with("Not run: the user declined this."));
    assert!(vw_ask::DECLINED.contains("Nothing was done"));
    assert!(vw_ask::DECLINED.contains("never as done"));
}

/// The public manuals quote the decline text the agent receives word for word.
///
/// PREDICT: fails while any of these manuals quotes another wording of the decline text.
#[test]
fn the_manuals_quote_the_current_decline_text() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for manual in [
        "README.md",
        "docs/public/BROWSER_CONTROL.md",
        "docs/public/DEBUG_API.md",
    ] {
        let text = std::fs::read_to_string(root.join(manual)).unwrap();
        let quotes: Vec<&str> = text
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|quote| quote.starts_with("Not run: the user declined this."))
            .collect();
        assert!(!quotes.is_empty(), "{manual} quotes the decline text");
        for quote in quotes {
            assert_eq!(quote, vw_ask::DECLINED, "{manual}");
        }
    }
}
