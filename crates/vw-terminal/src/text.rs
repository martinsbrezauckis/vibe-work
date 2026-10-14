//! Turning raw terminal bytes into text for readers that want text (the Debug API tail, the
//! browser dev client): cutting on UTF-8 character boundaries and, on request, dropping escape
//! sequences.
//!
//! Depends on: std. Called by: `flow` (reads), tests.
//! Invariants:
//! - `utf8_window` never splits a multi-byte character: a partial character at the front is
//!   skipped, one at the end is left for the next read (its bytes stay after `end`).
//! - [`strip_ansi`] removes CSI, OSC, DCS/SOS/PM/APC strings and two-byte escapes, and keeps text,
//!   newlines and tabs; it never interprets anything (an OSC 52 or OSC 8 sequence is just removed).

/// For `bytes` read at some offset: how many leading bytes to skip and how many to keep so the kept
/// range starts and ends on character boundaries. An incomplete trailing character is held back
/// (it is read again, whole, from the returned end). With at least 4 bytes the window always makes
/// progress (`skip + keep > 0`), which is why reads ask for at least 4.
#[must_use]
pub(crate) fn utf8_window(bytes: &[u8]) -> (usize, usize) {
    // Up to 3 continuation bytes can precede the first lead byte.
    let skip = bytes
        .iter()
        .take(3)
        .take_while(|b| is_continuation(**b))
        .count();
    let body = &bytes[skip..];
    (skip, body.len() - incomplete_tail(body))
}

const fn is_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

/// Bytes at the end of `bytes` that start a character whose remaining bytes have not arrived.
fn incomplete_tail(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(4) {
        let byte = bytes[bytes.len() - back];
        if is_continuation(byte) {
            continue;
        }
        let needed = match byte {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => return 0, // invalid lead byte: lossy decoding replaces it
        };
        return if back < needed { back } else { 0 };
    }
    0
}

/// Removes terminal escape sequences from `text`, keeping printable text, `\n`, `\r` and `\t`.
#[must_use]
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: parameters and intermediates, then one final byte in @..~.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC, DCS, SOS, PM, APC: a string ended by BEL or ST (ESC \).
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Character set selection takes one more character.
                Some('(' | ')' | '*' | '+') => {
                    chars.next();
                }
                // Any other two-byte escape (ESC 7, ESC =, ...).
                _ => {}
            },
            // C1 CSI (8-bit form).
            '\u{9b}' => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            '\n' | '\r' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}
