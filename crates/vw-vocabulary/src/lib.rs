//! Shared user-facing vocabulary for Rust dialogs and errors.
//!
//! Depends on the validated `vocabulary.json` generated at build time. Called by core and gate;
//! the UI imports that same JSON directly. Wire identifiers remain separate from display words.

#![forbid(unsafe_code)]

/// One named concept with its short explanation and control tooltip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Term {
    /// Visible name of the concept.
    pub name: &'static str,
    /// Plain-language description of the concept.
    pub description: &'static str,
    /// Tooltip or accessible explanation of a control for the concept.
    pub tooltip: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/vocabulary.rs"));

/// Looks up a concept by its shared group and wire identifier.
#[must_use]
pub fn term(group: &str, id: &str) -> Option<Term> {
    generated_term(group, id)
}

/// Returns the shared display name, preserving unknown identifiers verbatim.
#[must_use]
pub fn label<'a>(group: &str, id: &'a str) -> &'a str {
    term(group, id).map_or(id, |term| term.name)
}

/// Looks up a shared heading, sentence or template.
#[must_use]
pub fn text(id: &str) -> Option<&'static str> {
    generated_text(id)
}
