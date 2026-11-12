//! Bounds and rewrites Unified Harness compaction requests at the model proxy.
//! Depends on: `serde_json`. Called by: `server` for Mistral chat completions.
//! Vibe also uses `secondary_call` for vision. Compaction requests (streamed since Vibe 3.0.3) end in a
//! text-only user summary request after the historical images; vision requests carry the image in their
//! last user message. The built-in prompt also identifies image-free compaction for its deadline.

use serde_json::{Value, json};

/// A bounded chat request, including base64 images from several earlier turns.
pub(crate) const MAX_CHAT_BODY: usize = 128 * 1024 * 1024;

/// Shape and attachment payload metrics; no image data or prompt text is retained here.
#[derive(Default)]
pub(crate) struct ChatShape {
    pub(crate) session: Option<String>,
    pub(crate) input: Option<String>,
    pub(crate) compaction: bool,
    /// A side request ([`side`]): its answer's rate limits can be another model's than the chat's, so they do not
    /// replace the readout.
    pub(crate) side: bool,
    /// Only the new user input, never history or a follow-up after tool/assistant output.
    pub(crate) new_attachment: bool,
    pub(crate) images: usize,
    pub(crate) attachment_notes: usize,
    pub(crate) image_url_bytes: usize,
    pub(crate) omitted: usize,
}

/// A side request of Vibe's (`ChatShape::side`): the title or the smart-approve classifier (by their call type), or a
/// request that is not streamed and is no compaction (the turn summary). The chat's own turns are streamed, and its
/// compaction runs on the chat's model.
fn side(request: &Value, compaction: bool) -> bool {
    matches!(
        request
            .pointer("/metadata/call_type")
            .and_then(Value::as_str),
        Some("title_generation" | "smart_approve")
    ) || (request.get("messages").is_some()
        && request.get("stream") != Some(&Value::Bool(true))
        && !compaction)
}

fn is_attachment_note(text: &str) -> bool {
    let mut previous = "";
    for line in text.lines().map(str::trim_start) {
        if previous.starts_with("Attached file: ") && line.starts_with("Absolute path: ") {
            return true;
        }
        if line.starts_with("Attached image ") && line.contains(". Absolute path: ") {
            return true;
        }
        previous = line;
    }
    false
}

fn attribution(request: &Value, name: &str) -> Option<String> {
    request
        .get("metadata")?
        .get(name)?
        .as_str()
        .map(str::to_owned)
}

/// Inspect a provider request: explicit Off on known Mistral models and compaction image removal.
/// Other requests, including malformed JSON, are returned byte-for-byte.
pub(crate) fn prepare(bytes: &[u8]) -> (Option<Vec<u8>>, ChatShape) {
    let Ok(mut request) = serde_json::from_slice::<Value>(bytes) else {
        return (None, ChatShape::default());
    };
    let reasoning_fitted = crate::reasoning::fit(&mut request);
    let compaction_prompt = request
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            let last = messages.last()?;
            let content = last.get("content")?.as_str()?;
            if last.get("role")?.as_str()? != "user" {
                return None;
            }
            // Vibe 3.0.3 retries a summary that was cut off with this fixed last message
            // (`compaction.cut_off_retry` in the harness's default prompts).
            let builtin = (content.contains("You are performing a CONTEXT CHECKPOINT COMPACTION.")
                && content.contains("<summary>"))
                || content == "You were cut off. Don't think, don't be long, just answer.";
            let historical_image = messages[..messages.len() - 1].iter().any(|message| {
                message
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|parts| {
                        parts.iter().any(|part| {
                            part.pointer("/image_url/url")
                                .and_then(Value::as_str)
                                .is_some_and(|url| url.starts_with("data:image/"))
                        })
                    })
            });
            Some(builtin || historical_image)
        })
        .unwrap_or(false);
    // Streamed or not: Vibe 3.0.3 and later stream their compaction call; earlier versions did not.
    let mut shape = ChatShape {
        session: attribution(&request, "session_id"),
        input: attribution(&request, "message_id"),
        compaction: request
            .pointer("/metadata/call_type")
            .and_then(Value::as_str)
            == Some("secondary_call")
            && compaction_prompt,
        ..ChatShape::default()
    };
    shape.side = side(&request, shape.compaction);
    let Some(messages) = request.get_mut("messages").and_then(Value::as_array_mut) else {
        return (None, shape);
    };
    let last = messages.len().saturating_sub(1);
    for (index, message) in messages.iter_mut().enumerate() {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        if message
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(is_attachment_note)
        {
            shape.attachment_notes += 1;
            shape.new_attachment |= index == last;
        }
        let Some(parts) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts {
            if part
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(is_attachment_note)
            {
                shape.attachment_notes += 1;
                shape.new_attachment |= index == last;
            }
            if part.get("type").and_then(Value::as_str) != Some("image_url") {
                continue;
            }
            let Some(url) = part.pointer("/image_url/url").and_then(Value::as_str) else {
                continue;
            };
            if !url.starts_with("data:image/") {
                continue;
            }
            shape.new_attachment |= index == last;
            shape.images += 1;
            shape.image_url_bytes += url.len();
            if shape.compaction {
                *part = json!({
                    "type": "text",
                    "text": "[Image omitted from compaction input; its original message still contains it.]"
                });
                shape.omitted += 1;
            }
        }
    }
    if shape.omitted == 0 && !reasoning_fitted {
        return (None, shape);
    }
    (serde_json::to_vec(&request).ok(), shape)
}
