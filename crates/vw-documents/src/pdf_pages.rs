//! The page count of a PDF, without a PDF library (`lopdf` would add ~20
//! crates, encryption included, for one number shown before the user decides to spend OCR quota).
//!
//! Method: every object's text is looked at, both the plain objects (`... obj... endobj`) and the
//! objects packed inside `FlateDecode` object streams (`/Type /ObjStm`, PDF 1.5+, where modern writers
//! put the page tree). The answer is the largest `/Count` of a `/Type /Pages` object (the root of the
//! page tree counts every page); without one, the number of `/Type /Page` objects; else unknown.
//! An encrypted file's object streams do not inflate, so its count is usually unknown: Mistral then
//! counts the pages (the chip says so).
//!
//! Depends on: flate2. Called by: `pages`.
//! Invariants: bounded work and memory for any input, including a hostile file the user adds: at most
//! [`MAX_INFLATED`] bytes inflated per stream, [`MAX_TOTAL_INFLATED`] for the whole document, [`MAX_STREAMS`]
//! object streams and [`MAX_OBJECTS`] objects examined; packed objects are examined in place (borrowed slices of
//! one inflated stream at a time, never copied or kept), and a stream whose object offsets are not strictly
//! increasing and inside its body is skipped (overlapping offsets would otherwise re-read the same bytes many
//! times). Over a budget the count is `None` (unknown): never a guess from a partial scan. Never panics on damaged
//! input; the count is an estimate (an incrementally updated file may still hold an older, larger page tree).

use std::io::Read;

/// Most bytes one object stream may inflate to.
pub const MAX_INFLATED: u64 = 64 * 1024 * 1024;
/// Most bytes all object streams of one document may inflate to together.
pub const MAX_TOTAL_INFLATED: u64 = 256 * 1024 * 1024;
/// Most object streams looked at.
pub const MAX_STREAMS: usize = 2_000;
/// Most objects (plain and packed) examined in one document.
pub const MAX_OBJECTS: usize = 1_000_000;

/// True for bytes that end a PDF name (white space or a delimiter).
fn ends_name(b: Option<&u8>) -> bool {
    match b {
        None => true,
        Some(c) => c.is_ascii_whitespace() || b"/<>[]()%{}".contains(c),
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// True when the object text holds `/Type <value>` (`value` with its slash, e.g. `/Pages`).
fn has_type(object: &[u8], value: &[u8]) -> bool {
    let mut at = 0;
    while let Some(p) = find(object, b"/Type", at) {
        let mut i = p + 5;
        while object.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if object.get(i..i + value.len()) == Some(value) && ends_name(object.get(i + value.len())) {
            return true;
        }
        at = p + 5;
    }
    false
}

/// The integer after `/<key>` in the object text, if any.
fn int_after(object: &[u8], key: &[u8]) -> Option<u64> {
    let mut at = 0;
    while let Some(p) = find(object, key, at) {
        let mut i = p + key.len();
        at = i;
        if !ends_name(object.get(i)) {
            continue;
        }
        while object.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let start = i;
        while object.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i > start
            && let Some(n) = std::str::from_utf8(&object[start..i])
                .ok()
                .and_then(|s| s.parse().ok())
        {
            return Some(n);
        }
    }
    None
}

/// The plain objects: the file split at `endobj`.
fn plain_objects(pdf: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(p) = find(pdf, b"endobj", start) {
        out.push(&pdf[start..p]);
        start = p + 6;
    }
    out
}

/// The raw bytes between `stream` and `endstream` of one object.
fn stream_data(object: &[u8]) -> Option<&[u8]> {
    let s = find(object, b"stream", 0)?;
    let mut begin = s + 6;
    if object.get(begin) == Some(&b'\r') {
        begin += 1;
    }
    if object.get(begin) == Some(&b'\n') {
        begin += 1;
    }
    let end = object
        .windows(9)
        .rposition(|w| w == b"endstream")
        .filter(|&e| e >= begin)?;
    Some(&object[begin..end])
}

/// Inflates a `FlateDecode` stream to at most `limit` bytes. `None` when it does not inflate; `Some((bytes,
/// true))` when it reached the limit (the stream is larger than allowed: its content is not trusted).
fn inflate(data: &[u8], limit: u64) -> Option<(Vec<u8>, bool)> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(limit)
        .read_to_end(&mut out)
        .ok()?;
    let full = out.len() as u64 >= limit;
    Some((out, full))
}

/// The page-tree facts gathered so far, and the work budget left.
struct Tally {
    tree_count: Option<u64>,
    leaves: u64,
    objects: usize,
    inflated: u64,
    /// A budget ran out: the count is unknown.
    over: bool,
}

impl Tally {
    /// Looks at one object's text (plain or packed).
    fn object(&mut self, object: &[u8]) {
        self.objects += 1;
        if self.objects > MAX_OBJECTS {
            self.over = true;
            return;
        }
        if has_type(object, b"/Pages") {
            if let Some(count) = int_after(object, b"/Count") {
                self.tree_count = Some(self.tree_count.map_or(count, |c| c.max(count)));
            }
        } else if has_type(object, b"/Page") {
            self.leaves += 1;
        }
    }

    /// Looks at the objects packed inside one object stream (`/N` pairs of `number offset`, objects from `/First`),
    /// in place. A stream that does not inflate, or whose offsets are not strictly increasing inside its body, is
    /// skipped.
    fn object_stream(&mut self, object: &[u8]) {
        let dict_end = find(object, b"stream", 0).unwrap_or(object.len());
        let dict = &object[..dict_end];
        if !has_type(dict, b"/ObjStm") || find(dict, b"/FlateDecode", 0).is_none() {
            return;
        }
        let (Some(n), Some(first)) = (int_after(dict, b"/N"), int_after(dict, b"/First")) else {
            return;
        };
        let Ok(first) = usize::try_from(first) else {
            return;
        };
        let left = MAX_TOTAL_INFLATED.saturating_sub(self.inflated);
        if left == 0 {
            self.over = true;
            return;
        }
        let Some((data, full)) =
            stream_data(object).and_then(|raw| inflate(raw, MAX_INFLATED.min(left)))
        else {
            return;
        };
        self.inflated += data.len() as u64;
        if full {
            // Larger than one stream (or the rest of the document) may be: not looked at, and the count is unknown.
            self.over = true;
            return;
        }
        let header = String::from_utf8_lossy(data.get(..first).unwrap_or_default()).into_owned();
        let offsets: Vec<usize> = header
            .split_ascii_whitespace()
            .filter_map(|t| t.parse().ok())
            .skip(1)
            .step_by(2)
            .take(usize::try_from(n).unwrap_or(0).min(MAX_OBJECTS))
            .collect();
        let body = data.get(first..).unwrap_or_default();
        let ordered = offsets.windows(2).all(|w| w[0] < w[1])
            && offsets.last().is_none_or(|&last| last <= body.len());
        if !ordered {
            return;
        }
        for (i, &start) in offsets.iter().enumerate() {
            let end = offsets.get(i + 1).copied().unwrap_or(body.len());
            if let Some(packed) = body.get(start..end) {
                self.object(packed);
            }
            if self.over {
                return;
            }
        }
    }
}

/// The page count of the PDF in `pdf`, or `None` when it cannot be told (no page tree, or a work budget ran out).
#[must_use]
pub fn count_pages(pdf: &[u8]) -> Option<u32> {
    let plain = plain_objects(pdf);
    let mut tally = Tally {
        tree_count: None,
        leaves: 0,
        objects: 0,
        inflated: 0,
        over: false,
    };
    let mut streams = 0;
    for object in &plain {
        if has_type(object, b"/ObjStm") {
            streams += 1;
            if streams > MAX_STREAMS {
                break;
            }
            tally.object_stream(object);
        } else {
            tally.object(object);
        }
        if tally.over {
            return None;
        }
    }
    let found = tally
        .tree_count
        .or((tally.leaves > 0).then_some(tally.leaves))?;
    u32::try_from(found).ok().filter(|&n| n > 0)
}
