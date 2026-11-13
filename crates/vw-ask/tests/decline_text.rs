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
