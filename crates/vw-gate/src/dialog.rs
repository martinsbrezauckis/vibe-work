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
//! - [`dialog_label`] is for a name that a dialog sentence places between quotation marks of any style: hidden
//!   and quote-like characters ([`is_quote_char`]) are written as visible escapes, so the name can neither
//!   end the surrounding quotation early nor add a sentence after it.

/// True for characters a dialog never shows as they are (module docs).
#[must_use]
pub fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || ('\u{202A}'..='\u{202E}').contains(&c)
        || ('\u{2066}'..='\u{2069}').contains(&c)
        || ('\u{200B}'..='\u{200F}').contains(&c)
        || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}' | '\u{061C}')
}

/// True for characters that can open or close a quotation in some writing style: ASCII quotes and
/// backtick, guillemets, curly and low-9 quotes, primes, CJK corner brackets and full-width quotes, plus
/// a few look-alike quotation ornaments. Keep this set aligned with `QUOTE_LIKE` in
/// `ui/src/state/pluginText.ts`, which writes plugin labels in the app the same way.
#[must_use]
pub fn is_quote_char(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | '`' | '\u{00AB}' | '\u{00BB}' | '\u{02BA}' | '\u{02BC}' | '\u{02EE}' | '\u{05F4}'
            | '\u{2018}'..='\u{201F}'
            | '\u{2032}'..='\u{2037}'
            | '\u{2039}' | '\u{203A}' | '\u{2057}'
            | '\u{275B}'..='\u{275E}'
            | '\u{2E42}'
            | '\u{300C}'..='\u{300F}'
            | '\u{301D}'..='\u{301F}'
            | '\u{FF02}' | '\u{FF07}' | '\u{FF62}' | '\u{FF63}'
            | '\u{1F676}'..='\u{1F678}'
    )
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

/// A complete, visibly escaped name for a dialog sentence that puts it between quotation marks, without
/// the marks themselves. Hidden and quote-like characters become `\uXXXX` and a backslash becomes `\\`,
/// so every quotation mark a reader sees around the name belongs to the sentence, not to the name.
#[must_use]
pub fn dialog_label(text: &str) -> String {
    use std::fmt::Write as _;
    let mut label = String::with_capacity(text.len());
    for ch in text.chars() {
        if is_hidden_char(ch) || is_quote_char(ch) {
            let _ = write!(label, "\\u{:04X}", ch as u32);
        } else if ch == '\\' {
            label.push_str("\\\\");
        } else {
            label.push(ch);
        }
    }
    label
}

#[cfg(test)]
mod kg1_tests {
    use super::{dialog_label, dialog_quote};

    #[test]
    fn quote_preserves_identity_and_exposes_hidden_characters() {
        assert_eq!(
            dialog_quote("a  \"b\n\u{202E}c"),
            "\"a  \\\"b\\u000A\\u202Ec\""
        );
    }

    #[test]
    fn label_writes_every_quote_like_character_as_a_visible_escape() {
        let points = [
            0x22, 0x27, 0x60, 0xAB, 0xBB, 0x2BA, 0x2BC, 0x2EE, 0x5F4, 0x2018, 0x2019, 0x201A,
            0x201B, 0x201C, 0x201D, 0x201E, 0x201F, 0x2032, 0x2033, 0x2034, 0x2035, 0x2036, 0x2037,
            0x2039, 0x203A, 0x2057, 0x275B, 0x275C, 0x275D, 0x275E, 0x2E42, 0x300C, 0x300D, 0x300E,
            0x300F, 0x301D, 0x301E, 0x301F, 0xFF02, 0xFF07, 0xFF62, 0xFF63, 0x1F676, 0x1F677,
            0x1F678,
        ];
        for point in points {
            let ch = char::from_u32(point).unwrap();
            assert_eq!(
                dialog_label(&format!("a{ch}b")),
                format!("a\\u{point:04X}b"),
                "U+{point:04X}"
            );
        }
        assert_eq!(dialog_label("a\\b\n\u{202E}c d"), "a\\\\b\\u000A\\u202Ec d");
        assert_eq!(dialog_label("Review helper 🦊"), "Review helper 🦊");
    }

    #[test]
    fn label_leaves_no_quote_mark_that_could_end_the_sentence_quotation() {
        let name = "Trusted\u{201D} from the folder \"/safe/plugin\"? Its MCP servers are safe. \u{2018}x\u{2019} \u{201C}y";
        let label = dialog_label(name);
        assert_eq!(
            label,
            "Trusted\\u201D from the folder \\u0022/safe/plugin\\u0022? Its MCP servers are safe. \\u2018x\\u2019 \\u201Cy"
        );
        assert!(
            !label
                .chars()
                .any(|c| super::is_quote_char(c) || super::is_hidden_char(c))
        );
    }
}
