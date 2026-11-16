//! Where a character sits on a US keyboard, as a macOS virtual key: `WebKit` reports the page's `code` from the virtual
//! key and the page's `key` from the event's text. Shared by the Safari native input planner and the built-in
//! browser's macOS keyboard, so both browsers give a page the same `code`.

/// A virtual key that no Apple keyboard has (0x6C): `WebKit` gives it code "Unidentified" and takes `key` from the
/// event's text, and `AppKit` gives it no meaning of its own. Virtual key 0 would be the "a" key (code `KeyA`), and
/// 114 (a PC keyboard's Insert) is reported as key and code "Help".
pub(crate) const NO_KEY_POSITION: u16 = 0x6c;

/// The US-layout virtual key of a printable ASCII character (shifted characters share their key), or `None`.
pub(crate) fn us_key(name: &str) -> Option<u16> {
    Some(match name.to_ascii_lowercase().as_str() {
        "a" => 0,
        "s" => 1,
        "d" => 2,
        "f" => 3,
        "h" => 4,
        "g" => 5,
        "z" => 6,
        "x" => 7,
        "c" => 8,
        "v" => 9,
        "b" => 11,
        "q" => 12,
        "w" => 13,
        "e" => 14,
        "r" => 15,
        "y" => 16,
        "t" => 17,
        "1" | "!" => 18,
        "2" | "@" => 19,
        "3" | "#" => 20,
        "4" | "$" => 21,
        "6" | "^" => 22,
        "5" | "%" => 23,
        "=" | "+" => 24,
        "9" | "(" => 25,
        "7" | "&" => 26,
        "-" | "_" => 27,
        "8" | "*" => 28,
        "0" | ")" => 29,
        "]" | "}" => 30,
        "o" => 31,
        "u" => 32,
        "[" | "{" => 33,
        "i" => 34,
        "p" => 35,
        "l" => 37,
        "j" => 38,
        "'" | "\"" => 39,
        "k" => 40,
        ";" | ":" => 41,
        "\\" | "|" => 42,
        "," | "<" => 43,
        "/" | "?" => 44,
        "n" => 45,
        "m" => 46,
        "." | ">" => 47,
        "`" | "~" => 50,
        _ => return None,
    })
}

/// The virtual key a typed character is sent with: a letter or digit its US key (code `KeyQ`, `Digit7`, as the other
/// browsers report it), any other character no key position (code "Unidentified").
pub(crate) fn typed_character_key(name: &str) -> u16 {
    if name.chars().all(|c| c.is_ascii_alphanumeric()) {
        us_key(name).unwrap_or(NO_KEY_POSITION)
    } else {
        NO_KEY_POSITION
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn letters_and_digits_have_their_us_key_and_nothing_else_falls_back_to_the_a_key() {
        for (key, code) in [
            ("q", 12),
            ("Q", 12),
            ("a", 0),
            ("z", 6),
            ("7", 26),
            ("0", 29),
        ] {
            assert_eq!(typed_character_key(key), code, "{key}");
        }
        for key in [",", "/", "!", " ", "Ā", "🙂", "é", "Insert"] {
            assert_eq!(typed_character_key(key), NO_KEY_POSITION, "{key}");
        }
        assert_eq!(us_key("!"), Some(18), "a shifted character shares its key");
        assert_eq!(us_key("é"), None);
    }
}
