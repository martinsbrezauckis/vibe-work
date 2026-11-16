//! The image check's base64 decoding: standard alphabet, padding optional, nothing else.
//!
//! PREDICT (red before the change): the hand-written decoder accepted over-padded text ("QUI==", three characters
//! with two `=`), so `over_padding_is_refused` failed; the other cases already held. After: the workspace `base64`
//! crate decodes (standard alphabet, canonical or no padding), the same as the provider will read it.

use super::decode;

#[test]
fn standard_text_decodes_with_or_without_padding() {
    assert_eq!(decode("aGVsbG8=").as_deref(), Some(b"hello".as_slice()));
    assert_eq!(decode("aGVsbG8").as_deref(), Some(b"hello".as_slice()));
    assert_eq!(decode("aGk=").as_deref(), Some(b"hi".as_slice()));
    assert_eq!(decode("aGk").as_deref(), Some(b"hi".as_slice()));
    assert_eq!(decode("").as_deref(), Some(b"".as_slice()));
    assert_eq!(decode("+/8=").as_deref(), Some([0xfb, 0xff].as_slice()));
}

#[test]
fn other_text_is_refused() {
    for text in [
        "aGVsbG8===",
        "a",
        "aGVsb",
        "aG=k",
        "aGk*",
        "-_8=",
        "aGk= ",
        "\naGk=",
    ] {
        assert_eq!(decode(text), None, "{text:?}");
    }
}

#[test]
fn over_padding_is_refused() {
    assert_eq!(decode("QUI=="), None);
}
