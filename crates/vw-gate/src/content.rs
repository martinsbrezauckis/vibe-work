//! User content leaving through the gate: an **allowlist** of content block types, each
//! one validated; anything else is refused before it reaches Vibe.
//!
//! Why: Vibe reads an image block with `source.kind: "file"` from any path it is given and sends the
//! bytes to the model provider (a file source is stored in history as inline base64). A
//! script in the webview, or any Debug API client, could otherwise attach any readable file, or name a
//! local path the model is then pointed at. The composer reads pasted, dropped and picked files in the
//! webview (the user chose them; no path crosses the bridge) and sends their bytes, so no block may name
//! a local file.
//!
//! Applies to `turn/start`, `turn/steer` (`message: ContentBlock[]`, `userDisplayContent`) and every
//! entry of `session/turn/enqueue` / `session/turn/queue/replace` (`entries[].content:
//! SessionContentBlock[]`, `entries[].annotations`). Allowed blocks:
//! - `text` (turn and session): a string;
//! - `image`: turn: `attachment.source.kind` must be `inline`; session: `uri` must be a
//!   `data:<type>;base64,<data>` URI; the type PNG, JPEG, GIF or WebP, the base64 decodes, the first bytes
//!   match the type, at most [`MAX_IMAGE_BYTES`] decoded and [`MAX_IMAGES`] per message;
//! - `resource` (turn) with `kind: text` (a string) or `kind: blob` (an image, checked like one), and
//!   `embedded_resource` (session) with `text` or `blob` the same way: the `uri` must use the app's
//!   `vw:` scheme (inserted context, `vw://context/<n>`); a resource LINK (`kind: link`, `resource_link`)
//!   is refused: it names something for Vibe to fetch instead of carrying it;
//! - all text of one message (blocks, resources and the display payload) at most [`MAX_TEXT_BYTES`].
//!
//! Depends on: shape (`invalid`), error. Called by: `session_rules::adjust`.
//! Invariant: error texts never quote a value from the params (`error.rs`).

use serde_json::Value;

use crate::error::GateError;
use crate::shape::invalid;

/// Largest image accepted, decoded (the composer uses the same number).
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// Most images in one message (the composer uses the same number).
pub const MAX_IMAGES: usize = 4;
/// Most text in one message, in bytes: typed text, inserted context and the display payload together.
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

const TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Checks the user content of a call; methods without user content pass untouched.
pub(crate) fn check(method: &str, params: &Value) -> Result<(), GateError> {
    match method {
        "turn/start" | "turn/steer" => {
            let mut budget = Budget::default();
            blocks(method, params.get("message"), false, &mut budget)?;
            budget.json(method, params.get("userDisplayContent"))
        }
        "session/turn/enqueue" | "session/turn/queue/replace" => {
            for entry in params
                .get("entries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let mut budget = Budget::default();
                blocks(method, entry.get("content"), true, &mut budget)?;
                budget.json(method, entry.get("annotations"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// What one message has used so far.
#[derive(Default)]
struct Budget {
    images: usize,
    text: usize,
}

impl Budget {
    fn text(&mut self, method: &str, text: Option<&Value>) -> Result<(), GateError> {
        let text = text
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(method, "a text part must be a string"))?;
        self.add(method, text.len())
    }

    fn add(&mut self, method: &str, bytes: usize) -> Result<(), GateError> {
        self.text += bytes;
        if self.text > MAX_TEXT_BYTES {
            return Err(invalid(method, "a message carries more than 4 MB of text"));
        }
        Ok(())
    }

    /// Counts an optional JSON payload (display content, annotations) against the text budget.
    fn json(&mut self, method: &str, value: Option<&Value>) -> Result<(), GateError> {
        match value {
            None | Some(Value::Null) => Ok(()),
            Some(v) => self.add(method, v.to_string().len()),
        }
    }

    fn image(&mut self, method: &str) -> Result<(), GateError> {
        self.images += 1;
        if self.images > MAX_IMAGES {
            return Err(invalid(
                method,
                format!("a message can carry at most {MAX_IMAGES} images"),
            ));
        }
        Ok(())
    }
}

fn blocks(
    method: &str,
    content: Option<&Value>,
    session_blocks: bool,
    budget: &mut Budget,
) -> Result<(), GateError> {
    for block in content.and_then(Value::as_array).into_iter().flatten() {
        match (block.get("type").and_then(Value::as_str), session_blocks) {
            (Some("text"), _) => budget.text(method, block.get("text"))?,
            (Some("image"), false) => {
                budget.image(method)?;
                let (mime, data) = inline(method, block.get("attachment"))?;
                image(method, mime, data)?;
            }
            (Some("image"), true) => {
                budget.image(method)?;
                let (mime, data) = data_uri(method, block.get("uri").and_then(Value::as_str))?;
                image(method, mime, data)?;
            }
            (Some("resource"), false) => {
                let resource = block.get("resource");
                let field = |k: &str| resource.and_then(|r| r.get(k));
                app_uri(method, field("uri"))?;
                match field("kind").and_then(Value::as_str) {
                    Some("text") => budget.text(method, field("text"))?,
                    Some("blob") => blob(method, field("mediaType"), field("blob"), budget)?,
                    _ => return Err(invalid(method, "only text and blob resources are accepted")),
                }
            }
            (Some("embedded_resource"), true) => {
                app_uri(method, block.get("uri"))?;
                match (block.get("text"), block.get("blob")) {
                    (Some(text), None | Some(Value::Null)) if !text.is_null() => {
                        budget.text(method, Some(text))?;
                    }
                    (None | Some(Value::Null), Some(data)) if !data.is_null() => {
                        blob(method, block.get("mediaType"), Some(data), budget)?;
                    }
                    _ => {
                        return Err(invalid(
                            method,
                            "an embedded resource carries exactly one of text or blob",
                        ));
                    }
                }
            }
            _ => {
                return Err(invalid(
                    method,
                    "this content block type is not accepted (text, image and resources only)",
                ));
            }
        }
    }
    Ok(())
}

/// A resource's `uri` must use the app's own `vw:` scheme (it labels inserted context; never a path).
fn app_uri(method: &str, uri: Option<&Value>) -> Result<(), GateError> {
    match uri.and_then(Value::as_str) {
        Some(u) if u.starts_with("vw://") && u.len() <= 200 => Ok(()),
        _ => Err(invalid(
            method,
            "a resource must carry its content under a vw:// name (no file paths or links)",
        )),
    }
}

/// A blob resource: an image, checked like an image block.
fn blob(
    method: &str,
    media_type: Option<&Value>,
    data: Option<&Value>,
    budget: &mut Budget,
) -> Result<(), GateError> {
    budget.image(method)?;
    image(
        method,
        media_type.and_then(Value::as_str).unwrap_or_default(),
        data.and_then(Value::as_str).unwrap_or_default(),
    )
}

/// `(mimeType, base64)` of an inline `ImageAttachment`; a file source is refused.
fn inline<'a>(
    method: &str,
    attachment: Option<&'a Value>,
) -> Result<(&'a str, &'a str), GateError> {
    let source = attachment.and_then(|a| a.get("source"));
    if source.and_then(|s| s.get("kind")).and_then(Value::as_str) != Some("inline") {
        return Err(invalid(
            method,
            "images must be sent inline (a file path source is not accepted)",
        ));
    }
    let mime = attachment
        .and_then(|a| a.get("mimeType"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let data = source
        .and_then(|s| s.get("data"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok((mime, data))
}

/// `(mimeType, base64)` of a `data:<type>;base64,<data>` URI.
fn data_uri<'a>(method: &str, uri: Option<&'a str>) -> Result<(&'a str, &'a str), GateError> {
    let refuse = || invalid(method, "a queued image must be a base64 data: URI");
    let rest = uri
        .and_then(|u| u.strip_prefix("data:"))
        .ok_or_else(refuse)?;
    let (mime, data) = rest.split_once(";base64,").ok_or_else(refuse)?;
    Ok((mime, data))
}

fn image(method: &str, mime: &str, data: &str) -> Result<(), GateError> {
    if !TYPES.contains(&mime) {
        return Err(invalid(
            method,
            "only PNG, JPEG, GIF and WebP images are accepted",
        ));
    }
    let bytes =
        decode(data).ok_or_else(|| invalid(method, "an image's base64 data does not decode"))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(invalid(method, "an image is larger than 10 MB"));
    }
    if sniff(&bytes) != Some(mime) {
        return Err(invalid(
            method,
            "an image's bytes do not match its declared type",
        ));
    }
    Ok(())
}

/// The image type `bytes` start with.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        Some("image/webp")
    } else {
        None
    }
}

/// Standard base64 (RFC 4648, padding optional); `None` on any other character or an impossible length.
/// Hand-written to keep the gate free of a new dependency for 30 lines.
fn decode(text: &str) -> Option<Vec<u8>> {
    let body = text.trim_end_matches('=');
    if text.len() - body.len() > 2 || body.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 2);
    let (mut acc, mut bits) = (0_u32, 0_u32);
    for c in body.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
        }
        acc &= (1 << bits) - 1;
    }
    Some(out)
}
