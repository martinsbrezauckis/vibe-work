//! Whether a document's HTML source may attach a closed shadow root through the parser (declarative shadow DOM), for
//! engines whose private script world cannot enter closed roots (`WebKitGTK`; macOS before 27). Their protected-value
//! admission refuses a document whose source this flags (vw-browser `protected_capture`).
//!
//! The rule matches the page-world keeper's (`js/closed_roots.js`): a `shadowrootmode` attribute (ASCII case folded)
//! not given exactly the plain value `open`, or the bare legacy `shadowroot` attribute, counts (other `shadowroot...`
//! attributes such as `shadowrootclonable` do not), so an entity-encoded or unusual value, the legacy
//! `shadowroot` attribute, or an attribute split by the document's encoding errs toward refusing. NUL bytes are
//! dropped first, so a UTF-16 document's ASCII text is read too. Depends on nothing. Called by the engine adapters.

/// The attribute-name prefix every declarative shadow root attribute starts with.
const NAME: &[u8] = b"shadowroot";

/// Sources larger than this are taken as holding one (not scanned).
pub const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;

/// Web archives (every frame's source and the page's subresources) larger than this are taken as holding one.
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;

/// True when `source` may make a closed declarative shadow root (or is over [`MAX_SOURCE_BYTES`]).
#[must_use]
pub fn declarative_closed_source(source: &[u8]) -> bool {
    source.len() > MAX_SOURCE_BYTES || scan(source)
}

/// [`declarative_closed_source`] over a whole web archive (macOS): its frames' document sources are stored as
/// received; a subresource's text that names the attribute only makes the answer stricter.
#[must_use]
pub fn declarative_closed_archive(archive: &[u8]) -> bool {
    archive.len() > MAX_ARCHIVE_BYTES || scan(archive)
}

fn scan(source: &[u8]) -> bool {
    let text: Vec<u8> = source
        .iter()
        .filter(|b| **b != 0)
        .map(u8::to_ascii_lowercase)
        .collect();
    let mut at = 0;
    while let Some(found) = find(&text[at..], NAME) {
        let start = at + found;
        if !other_attribute(&text[start + NAME.len()..]) && !plain_open(&text[start..]) {
            return true;
        }
        at = start + NAME.len();
    }
    false
}

/// `shadowroot` begins another template attribute (`shadowrootclonable`, `shadowrootdelegatesfocus`,
/// `shadowrootserializable`, ...): a letter other than the `m` of `mode` follows.
fn other_attribute(after: &[u8]) -> bool {
    after
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() && *b != b'm')
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// `shadowrootmode` `=` `open` (bare, or in matching quotes) and the bare value ends there.
fn plain_open(text: &[u8]) -> bool {
    let space = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\x0c' | b'\r');
    let Some(rest) = text.strip_prefix(b"shadowrootmode") else {
        return false;
    };
    let rest = &rest[rest.iter().take_while(|b| space(b)).count()..];
    let Some(rest) = rest.strip_prefix(b"=") else {
        return false;
    };
    let rest = &rest[rest.iter().take_while(|b| space(b)).count()..];
    if rest.starts_with(b"\"open\"") || rest.starts_with(b"'open'") {
        return true;
    }
    rest.strip_prefix(b"open")
        .and_then(|after| after.first())
        .is_some_and(|b| space(b) || *b == b'>' || *b == b'/')
}

#[cfg(test)]
mod tests {
    use super::declarative_closed_source as risky;

    #[test]
    fn closed_and_unclear_declarative_roots_count_and_plain_open_ones_do_not() {
        assert!(risky(
            b"<div><template shadowrootmode=\"closed\"><input></template></div>"
        ));
        assert!(risky(
            b"<div><TEMPLATE SHADOWROOTMODE=CLOSED></TEMPLATE></div>"
        ));
        assert!(
            risky(b"<template shadowrootmode=\"&#99;losed\">"),
            "an entity may spell closed"
        );
        assert!(
            risky(b"<template shadowroot=closed>"),
            "the legacy attribute"
        );
        assert!(risky(b"<template shadowrootmode>"));
        assert!(risky(b"<template shadowrootmode=opener>"));
        let utf16: Vec<u8> = "<template shadowrootmode=closed>"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(risky(&utf16), "UTF-16 source");
        assert!(risky(&vec![b' '; super::MAX_SOURCE_BYTES + 1]));
        assert!(!risky(
            b"<div><template shadowrootmode=\"open\"><input></template></div>"
        ));
        assert!(!risky(
            b"<template shadowrootmode = 'open' shadowrootdelegatesfocus>"
        ));
        assert!(!risky(b"<template shadowrootmode=open>"));
        assert!(!risky(
            b"<p>An ordinary page</p><template><p>inert</p></template>"
        ));
        assert!(!risky(b""));
    }
}
