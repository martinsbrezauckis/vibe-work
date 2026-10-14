//! A small, tolerant HTML tag tokenizer for reading page headers, plan steps, section ids and links out of
//! agent-written pages. It is not a renderer and never produces HTML: it only reads.
//!
//! Depends on: nothing. Called by: `page` (page facts), `handbook::head` (head-only reads).
//! Invariants:
//! - Never panics on any input; works on at most [`MAX_INPUT`] bytes (the rest of a page is ignored).
//! - Comments, doctypes and processing instructions are skipped; the content of `script` is skipped entirely and the
//!   content of `style` comes back as [`Token::Style`] (never as text or tags: a `<meta` inside either is not a
//!   header); `title` and `textarea` content is text.
//! - Tag and attribute names are lowercased; attribute values are entity-decoded.

/// Bytes of a page that are read at most (a page larger than this is read in part).
pub const MAX_INPUT: usize = 256 * 1024;

/// Elements without content (no closing tag expected).
pub const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// One tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    /// Lowercased name.
    pub name: String,
    /// Attributes in order (lowercased names, decoded values; a bare attribute has an empty value).
    pub attrs: Vec<(String, String)>,
    /// `</name>`.
    pub closing: bool,
    /// `<name/>` or a void element.
    pub self_closing: bool,
}

impl Tag {
    /// The value of attribute `name`, if present.
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A token of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// A start or end tag.
    Tag(Tag),
    /// Text between tags, entity-decoded.
    Text(String),
    /// The raw content of a `<style>` element (not decoded; read for `@import` and `url()` only).
    Style(String),
}

/// Splits `html` into tags and text (see the module invariants).
#[must_use]
pub fn tokens(html: &str) -> Vec<Token> {
    let text = clip_to_boundary(html, MAX_INPUT);
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        // Markup that is not a tag: comments, doctype, processing instructions.
        if rest.starts_with("<!--") {
            push_text(&mut out, &text[text_start..i]);
            i = rest.find("-->").map_or(bytes.len(), |end| i + end + 3);
            text_start = i;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            push_text(&mut out, &text[text_start..i]);
            i = rest.find('>').map_or(bytes.len(), |end| i + end + 1);
            text_start = i;
            continue;
        }
        let Some((tag, len)) = parse_tag(rest) else {
            i += 1;
            continue;
        };
        push_text(&mut out, &text[text_start..i]);
        i += len;
        text_start = i;
        let raw = !tag.closing && matches!(tag.name.as_str(), "script" | "style");
        let rcdata = !tag.closing && matches!(tag.name.as_str(), "title" | "textarea");
        let name = tag.name.clone();
        out.push(Token::Tag(tag));
        if raw || rcdata {
            // Content runs to the matching end tag (case-insensitive); none means to the end.
            let end = find_end_tag(&text[i..], &name).map_or(bytes.len(), |e| i + e);
            if rcdata {
                push_text(&mut out, &text[i..end]);
            } else if name == "style" && end > i {
                out.push(Token::Style(text[i..end].to_owned()));
            }
            i = end;
            text_start = i;
        }
    }
    push_text(&mut out, &text[text_start..]);
    out
}

/// Cuts `text` to at most `max` bytes on a character boundary.
fn clip_to_boundary(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn push_text(out: &mut Vec<Token>, raw: &str) {
    if !raw.is_empty() {
        out.push(Token::Text(decode_entities(raw)));
    }
}

/// Offset of `</name` (ASCII case-insensitive) in `text`.
pub(crate) fn find_end_tag(text: &str, name: &str) -> Option<usize> {
    let needle = format!("</{name}");
    let lower = text.to_ascii_lowercase();
    lower.find(&needle)
}

/// Parses one tag at the start of `text` (which starts with `<`). Returns the tag and its length in bytes, or
/// `None` when this `<` does not start a tag (then it is text).
pub(crate) fn parse_tag(text: &str) -> Option<(Tag, usize)> {
    let bytes = text.as_bytes();
    let mut i = 1;
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    let name_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
        i += 1;
    }
    if i == name_start || !bytes[name_start].is_ascii_alphabetic() {
        return None;
    }
    let name = text[name_start..i].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            None => return None,
            Some(b'>') => {
                i += 1;
                break;
            }
            Some(b'/') => {
                self_closing = true;
                i += 1;
                continue;
            }
            Some(_) => {}
        }
        let key_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b'=' | b'>' | b'/')
        {
            i += 1;
        }
        let key = text[key_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            match bytes.get(i) {
                Some(&q @ (b'"' | b'\'')) => {
                    let start = i + 1;
                    let end = text[start..].find(q as char).map(|e| start + e)?;
                    value = decode_entities(&text[start..end]);
                    i = end + 1;
                }
                Some(_) => {
                    let start = i;
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                        i += 1;
                    }
                    value = decode_entities(&text[start..i]);
                }
                None => return None,
            }
        }
        if !key.is_empty() {
            attrs.push((key, value));
        }
    }
    let self_closing = self_closing || VOID.contains(&name.as_str());
    Some((
        Tag {
            name,
            attrs,
            closing,
            self_closing,
        },
        i,
    ))
}

/// Decodes the common named entities and numeric references; anything unknown stays as written.
#[must_use]
pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let end = rest.find(';').filter(|&e| e <= 12);
        let decoded = end.and_then(|e| entity(&rest[1..e]));
        if let (Some(e), Some(c)) = (end, decoded) {
            out.push(c);
            rest = &rest[e + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        "mdash" => Some('—'),
        "ndash" => Some('–'),
        "hellip" => Some('…'),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code).filter(|c| *c != '\0')
        }
    }
}

/// Escapes text for HTML element content and quoted attribute values.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Collapses whitespace runs to one space and trims.
#[must_use]
pub fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// At most `max` characters of `text`.
#[must_use]
pub fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}
