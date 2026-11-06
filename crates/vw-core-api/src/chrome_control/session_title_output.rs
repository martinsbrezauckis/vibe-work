//! Remove private host session titles at the model boundary, including older helpers' text-only results.
//! Called by content before prompt marking and output caching; page/tab titles remain page data.
use serde_json::Value;

pub(super) fn strip(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if object.get("sessionId").is_some_and(Value::is_string)
                && object.get("state").is_some_and(Value::is_string)
            {
                object.remove("title");
            }
            // These are helper session-view locations, including failed opens and repeated marking.
            // Never descend into execution data or tab/page metadata just because it resembles a session.
            for key in ["sessions", "pending", "task", "untrustedPageContent"] {
                if let Some(child) = object.get_mut(key) {
                    strip(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                // Legacy MCP content has JSON inside a text block instead of structuredContent.
                if item["type"] == "text"
                    && let Some(text) = item["text"].as_str()
                    && let Ok(mut payload) = serde_json::from_str::<Value>(text)
                {
                    strip(&mut payload);
                    item["text"] = Value::String(payload.to_string());
                } else {
                    strip(item);
                }
            }
        }
        _ => {}
    }
}
