//! Bounds and rewrites Unified Harness compaction requests at the model proxy.
//! Depends on: `serde_json`. Called by: `server` for Mistral chat completions.
//! Vibe also uses nonstreaming `secondary_call` for vision. Compaction requests end in a text-only
//! user summary request after the historical images; vision requests carry the image in their last
//! user message. The built-in prompt also identifies image-free compaction for its deadline.

use serde_json::{Value, json};

/// A bounded chat request, including base64 images from several earlier turns.
pub(crate) const MAX_CHAT_BODY: usize = 128 * 1024 * 1024;

/// Shape and attachment payload metrics; no image data or prompt text is retained here.
#[derive(Default)]
pub(crate) struct ChatShape {
    pub(crate) compaction: bool,
    /// Only the new user input, never history or a follow-up after tool/assistant output.
    pub(crate) new_attachment: bool,
    pub(crate) images: usize,
    pub(crate) attachment_notes: usize,
    pub(crate) image_url_bytes: usize,
    pub(crate) omitted: usize,
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

/// Inspect a provider request: explicit Off on known Mistral models and compaction image removal.
/// Other requests, including malformed JSON, are returned byte-for-byte.
pub(crate) fn prepare(bytes: &[u8]) -> (Option<Vec<u8>>, ChatShape) {
    let Ok(mut request) = serde_json::from_slice::<Value>(bytes) else {
        return (None, ChatShape::default());
    };
    let explicit_off = crate::reasoning::explicit_off(&mut request);
    let compaction_prompt = request
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            let last = messages.last()?;
            let content = last.get("content")?.as_str()?;
            if last.get("role")?.as_str()? != "user" {
                return None;
            }
            let builtin = content.contains("You are performing a CONTEXT CHECKPOINT COMPACTION.")
                && content.contains("<summary>");
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
    let mut shape = ChatShape {
        compaction: request.get("stream") == Some(&Value::Bool(false))
            && request
                .pointer("/metadata/call_type")
                .and_then(Value::as_str)
                == Some("secondary_call")
            && compaction_prompt,
        ..ChatShape::default()
    };
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
    if shape.omitted == 0 && !explicit_off {
        return (None, shape);
    }
    (serde_json::to_vec(&request).ok(), shape)
}

#[cfg(test)]
mod tests {
    use super::prepare;
    use serde_json::{Value, json};

    fn request(images: usize, stream: bool, call_type: &str) -> Vec<u8> {
        let parts = (0..images)
            .map(|_| json!({"type":"image_url","image_url":{"url":"data:image/png;base64,aGVsbG8="}}))
            .collect::<Vec<_>>();
        serde_json::to_vec(&json!({
            "stream":stream,"metadata":{"call_type":call_type},
            "messages":[
                {"role":"user","content":parts},
                {"role":"user","content":"You are performing a CONTEXT CHECKPOINT COMPACTION. Wrap it in <summary> tags."}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn omits_two_three_and_five_images_only_for_compaction() {
        for count in [2, 3, 5] {
            let bytes = request(count, false, "secondary_call");
            let (rewritten, shape) = prepare(&bytes);
            assert!(shape.compaction);
            assert_eq!(shape.images, count);
            assert_eq!(shape.omitted, count);
            let rewritten = rewritten.unwrap();
            assert!(!String::from_utf8_lossy(&rewritten).contains("base64"));
            let value: Value = serde_json::from_slice(&rewritten).unwrap();
            assert_eq!(
                value["messages"][0]["content"].as_array().unwrap().len(),
                count
            );
            assert!(
                value["messages"][0]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("original message")
            );
            for (stream, call_type) in [(true, "secondary_call"), (false, "main_call")] {
                let bytes = request(count, stream, call_type);
                let (unchanged, shape) = prepare(&bytes);
                assert!(unchanged.is_none());
                assert_eq!(shape.images, count);
                assert_eq!(shape.omitted, 0);
            }
        }
    }

    #[test]
    fn unrelated_and_malformed_parts_survive() {
        let bytes = serde_json::to_vec(&json!({
            "stream":false,"metadata":{"call_type":"secondary_call"},
            "messages":[{"role":"user","content":[
                {"type":"text","text":"keep this"},
                {"type":"image_url","image_url":{"url":"https://example.invalid/x.png"}},
                {"type":"image_url","image_url":{"url":"data:image/png;base64,AA=="}}
            ]},{"role":"user","content":"You are performing a CONTEXT CHECKPOINT COMPACTION. Wrap it in <summary> tags."}]
        }))
        .unwrap();
        let (rewritten, shape) = prepare(&bytes);
        assert_eq!(shape.omitted, 1);
        let value: Value = serde_json::from_slice(&rewritten.unwrap()).unwrap();
        assert_eq!(value["messages"][0]["content"][0]["text"], "keep this");
        assert_eq!(
            value["messages"][0]["content"][1]["image_url"]["url"],
            "https://example.invalid/x.png"
        );
        assert!(prepare(b"not json").0.is_none());
    }

    #[test]
    fn vision_secondary_call_keeps_its_images() {
        let bytes = serde_json::to_vec(&json!({
            "stream": false,
            "metadata": {"call_type": "secondary_call"},
            "messages": [{"role": "user", "content": [
                {"type":"text","text":"Describe this image"},
                {"type":"image_url","image_url":{"url":"data:image/png;base64,AA=="}}
            ]}]
        }))
        .unwrap();
        let (rewritten, shape) = prepare(&bytes);
        assert!(!shape.compaction);
        assert_eq!(shape.images, 1);
        assert_eq!(shape.omitted, 0);
        assert!(rewritten.is_none());
    }

    #[test]
    fn native_or_custom_compaction_ends_with_a_text_summary_request() {
        let bytes = serde_json::to_vec(&json!({
            "stream": false,
            "metadata": {"call_type": "secondary_call"},
            "messages": [
                {"role": "user", "content": [
                    {"type":"text","text":"What is here?"},
                    {"type":"image_url","image_url":{"url":"data:image/png;base64,AA=="}}
                ]},
                {"role": "user", "content": "Summarize the conversation for the next turn."}
            ]
        }))
        .unwrap();
        let (rewritten, shape) = prepare(&bytes);
        assert!(shape.compaction);
        assert_eq!(shape.omitted, 1);
        assert!(rewritten.is_some());
    }

    #[test]
    fn secondary_agent_followup_ending_in_a_tool_message_keeps_its_image() {
        let bytes = serde_json::to_vec(&json!({
            "stream": false,
            "metadata": {"call_type": "secondary_call"},
            "messages": [
                {"role": "user", "content": [
                    {"type":"image_url","image_url":{"url":"data:image/png;base64,AA=="}}
                ]},
                {"role": "tool", "content": "The tool finished."}
            ]
        }))
        .unwrap();
        let (rewritten, shape) = prepare(&bytes);
        assert!(!shape.compaction);
        assert_eq!(shape.omitted, 0);
        assert!(rewritten.is_none());
    }

    #[test]
    fn only_the_request_bringing_the_attachment_is_bounded() {
        for content in [
            json!("Attached file: report.pdf\nAbsolute path: /tmp/report.pdf"),
            json!([{"type":"image_url","image_url":{"url":"data:image/png;base64,AA=="}}]),
        ] {
            let mut body = json!({"stream":true,"messages":[{"role":"user","content":content}]});
            assert!(
                prepare(&serde_json::to_vec(&body).unwrap())
                    .1
                    .new_attachment
            );
            for followup in [
                json!({"role":"assistant","content":"I will read it."}),
                json!({"role":"tool","content":"Document text"}),
                json!({"role":"user","content":"Now write a long answer."}),
            ] {
                body["messages"].as_array_mut().unwrap().push(followup);
                assert!(
                    !prepare(&serde_json::to_vec(&body).unwrap())
                        .1
                        .new_attachment
                );
            }
        }
    }

    #[test]
    fn attachment_notes_are_recognized_only_in_user_text() {
        let bytes = serde_json::to_vec(&json!({
            "stream": true,
            "messages": [
                {"role": "user", "content": "Attached file: notes.txt\nAbsolute path: /tmp/notes.txt"},
                {"role": "user", "content": [
                    {"type": "text", "text": "Attached image \"plot.png\". Absolute path: /tmp/plot.png"}
                ]},
                {"role": "assistant", "content": "Attached file: fake\nAbsolute path: fake"},
                {"role": "user", "content": "Discuss the words Attached file: and Absolute path: here."}
            ]
        }))
        .unwrap();
        let (rewritten, shape) = prepare(&bytes);
        assert!(rewritten.is_none());
        assert!(!shape.new_attachment);
        assert_eq!(shape.attachment_notes, 2);
        assert_eq!(shape.images, 0);
    }
}
