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
//!   quotes and ambiguous backslashes. Use it when no JSON or raw params accompany the consent sentence.
//! - [`dialog_label`] is for a name that a dialog sentence places between quotation marks of any style: hidden
//!   and quote-like characters ([`is_quote_char`]) are written as visible escapes, so the name can neither
//!   end the surrounding quotation early nor add a sentence after it.

/// True for characters a dialog never shows as they are (module docs).
#[must_use]
pub fn is_hidden_char(c: char) -> bool {
    vw_ask::is_hidden_char(c)
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
/// Ordinary path separators stay literal. Backslash runs double only before a visible escape, a quote,
/// a hidden character (which becomes an escape), or the closing quotation mark. This is display text, not JSON.
#[must_use]
pub fn dialog_quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    escape_into(&mut quoted, text, Quotes::Ascii);
    quoted.push('"');
    quoted
}

/// A chat name in a native heading. Embedded ASCII quotes stay as typed inside curly outer quotes;
/// other names keep the usual quotation. Hidden characters and ambiguous backslashes use [`dialog_quote`]'s rules.
#[must_use]
pub fn dialog_chat_quote(text: &str) -> String {
    if !text.contains('"') {
        return dialog_quote(text);
    }
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('“');
    escape_into(&mut quoted, text, Quotes::Curly);
    quoted.push('”');
    quoted
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quotes {
    None,
    Ascii,
    Curly,
}

/// A complete command for the block a dialog ends with (`Command:` and the command on the next line): as typed,
/// quotes included, so the person reads what will run (a `\"` is part of the command), with hidden
/// characters (line breaks too) as visible `\uXXXX` escapes on one line. A backslash run doubles only before an
/// escape or a hidden character, so a typed `\u202E` and a real U+202E still differ. Only for a value no
/// sentence text follows on its line.
#[must_use]
pub fn dialog_command(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    escape_into(&mut shown, text, Quotes::None);
    shown
}

/// Native display escaping, appended to `out`; the chosen delimiters and hidden characters stay unambiguous.
fn escape_into(out: &mut String, text: &str, quotes: Quotes) {
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if is_hidden_char(ch) || (quotes == Quotes::Curly && matches!(ch, '“' | '”')) {
            use std::fmt::Write as _;
            let _ = write!(out, "\\u{:04X}", ch as u32);
        } else {
            match ch {
                '"' if quotes == Quotes::Ascii => out.push_str("\\\""),
                '\\' => {
                    let mut count = 1;
                    while chars.as_str().starts_with('\\') {
                        chars.next();
                        count += 1;
                    }
                    let rest = chars.as_str();
                    let unicode_escape = rest.strip_prefix('u').is_some_and(|tail| {
                        tail.as_bytes()
                            .get(..4)
                            .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
                    });
                    let ambiguous = rest.chars().next().map_or(quotes != Quotes::None, |next| {
                        (quotes == Quotes::Ascii && next == '"')
                            || (quotes == Quotes::Curly && matches!(next, '“' | '”'))
                            || is_hidden_char(next)
                    }) || unicode_escape;
                    // Treat a whole run alike: a literal escape and an escaped character must differ.
                    out.extend(std::iter::repeat_n(
                        '\\',
                        count * if ambiguous { 2 } else { 1 },
                    ));
                }
                _ => out.push(ch),
            }
        }
    }
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
