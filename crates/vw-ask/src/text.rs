//! One set of plain agent sentences. The core scrubs notes before calling these pure formatters.
use crate::Outcome;

/// True for characters that are invisible or change how the text around them reads: controls, bidirectional
/// embeddings, overrides and isolates, zero-width characters, line/paragraph separators, the byte-order mark and
/// the Arabic letter mark. Shared with the gate's confirmation text (`vw_gate::is_hidden_char`).
#[must_use]
pub fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || ('\u{202A}'..='\u{202E}').contains(&c)
        || ('\u{2066}'..='\u{2069}').contains(&c)
        || ('\u{200B}'..='\u{200F}').contains(&c)
        || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}' | '\u{061C}')
}

/// Trim plain feedback and render hidden characters ([`is_hidden_char`]) visibly. No Markdown interpretation is
/// applied. The 2,000-character limit applies to the visible result, so escaped characters cannot grow it.
#[must_use]
pub fn plain_note(note: &str) -> String {
    note.trim()
        .chars()
        .flat_map(|c| {
            let escaped: Vec<char> = if is_hidden_char(c) && c != '\n' {
                c.escape_default().collect()
            } else {
                vec![c]
            };
            escaped
        })
        .take(2000)
        .collect()
}
/// The outcome sentence; permission never adds text to an allowed tool's normal result.
#[must_use]
pub fn agent_text(outcome: &Outcome, what: &str) -> String {
    match outcome {
        Outcome::AnswerNotConfirmed { .. } => {
            format!("Vibe did not confirm the answer to {what}. The agent may still be waiting.")
        }
        Outcome::Allowed { .. } => String::new(),
        Outcome::Refused { note, .. } => {
            let mut text = format!("Not done: the user said No to {what}.");
            if let Some(note) = note.as_deref().filter(|n| !n.trim().is_empty()) {
                text.push_str(" The user's note: ");
                text.push_str(&plain_note(note));
            }
            text
        }
        Outcome::Cancelled { reason } => format!("Not done: {reason} before the user answered."),
        Outcome::Never => {
            format!("Not done: the user set {what} to Never in Vibe Work Settings.")
        }
    }
}
