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
/// The result of a call the user said No to on a Vibe Work card or window. The agent's turn goes on with it, so
/// the agent must explain why the delegated task needs it before asking again.
pub const DECLINED: &str =
    "Not run: the user declined this. Explain why it is needed for the user's delegated task.";

/// The outcome sentence; permission never adds text to an allowed tool's normal result. A No is always
/// [`DECLINED`], with no note or subject appended.
#[must_use]
pub fn agent_text(outcome: &Outcome, what: &str) -> String {
    match outcome {
        Outcome::AnswerNotConfirmed { .. } => {
            format!(
                "The answer to {what} could not be confirmed. Ask the user again if it is still needed."
            )
        }
        Outcome::Allowed { .. } => String::new(),
        Outcome::Refused { .. } => DECLINED.to_owned(),
        Outcome::Cancelled { reason } => {
            format!("Not done: {reason}. Do not run this action unless the user asks again.")
        }
        Outcome::Never => {
            format!(
                "Not done: {what} is {} in Vibe Work Settings. Ask the user to do this action.",
                vw_vocabulary::label("vaultPermissions", "never")
            )
        }
    }
}
