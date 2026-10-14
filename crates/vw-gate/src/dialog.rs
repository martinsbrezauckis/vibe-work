//! Text that goes into a native confirmation dialog: legacy compact values and complete quoted values.
//!
//! Depends on: nothing. Called by: vw-core-api (dialog sentences), vw-host (`dialog_text`, defence in depth).
//! Invariants:
//! - Legacy [`dialog_value`] holds no control character (newlines included), no Unicode line or paragraph
//!   separator, no bidirectional override or isolate (U+202A-202E, U+2066-2069), no zero-width or
//!   direction mark (U+200B-200F) and no byte-order mark: each becomes one space, runs of spaces collapse.
//!   So a name cannot start a new line ("... Click Cancel to continue") or reorder the text around it.
//! - [`dialog_value`] caps the length (an ellipsis marks the cut) and puts the value in double quotes;
//!   a double quote inside becomes a single quote, so the value cannot close its quotes early.
//! - [`dialog_quote`] preserves the complete identity of a value with visible escapes for hidden characters,
//!   backslashes and quotes. Use it when no JSON or raw params are shown alongside the consent sentence.

/// True for characters a dialog never shows as they are (module docs).
#[must_use]
pub fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || ('\u{202A}'..='\u{202E}').contains(&c)
        || ('\u{2066}'..='\u{2069}').contains(&c)
        || ('\u{200B}'..='\u{200F}').contains(&c)
        || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}' | '\u{061C}')
}

/// `text` as one plain line: hidden characters become spaces, runs of whitespace one space, trimmed.
#[must_use]
pub fn dialog_line(text: &str) -> String {
    let mapped: String = text
        .chars()
        .map(|c| if is_hidden_char(c) { ' ' } else { c })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// An untrusted value for a dialog sentence: [`dialog_line`], at most `max` characters (then `…`), in
/// double quotes.
#[must_use]
pub fn dialog_value(text: &str, max: usize) -> String {
    let line = dialog_line(text).replace('"', "'");
    let cut: String = line.chars().take(max).collect();
    if cut.chars().count() < line.chars().count() {
        format!("\"{cut}…\"")
    } else {
        format!("\"{cut}\"")
    }
}

/// A complete, visibly escaped value for a plain native confirmation sentence. Spaces and quotes remain
/// distinguishable, while control and direction characters are written as Unicode escapes on one line.
#[must_use]
pub fn dialog_quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for ch in text.chars() {
        if is_hidden_char(ch) {
            use std::fmt::Write as _;
            let _ = write!(quoted, "\\u{:04X}", ch as u32);
        } else {
            match ch {
                '"' => quoted.push_str("\\\""),
                '\\' => quoted.push_str("\\\\"),
                _ => quoted.push(ch),
            }
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod kg1_tests {
    use super::dialog_quote;

    #[test]
    fn quote_preserves_identity_and_exposes_hidden_characters() {
        assert_eq!(
            dialog_quote("a  \"b\n\u{202E}c"),
            "\"a  \\\"b\\u000A\\u202Ec\""
        );
    }
}
