//! Translate the shared page-key allowlist into the built-in browser's DOM event payload.
//! Admission belongs to vw-ask; called by the action request assembler after argument validation.
use serde_json::{Value, json};

pub(crate) fn request(chord: &str) -> Option<Value> {
    let (key, modifiers) = vw_ask::chrome::keys::canonical(chord, &[]).ok()?;
    Some(json!({
        "key": if key == "Space" { " " } else { &key },
        "modifiers": {
            "ctrlKey": modifiers.contains(&"Control"),
            "shiftKey": modifiers.contains(&"Shift"),
            "altKey": modifiers.contains(&"Alt"),
            "metaKey": modifiers.contains(&"Meta")
        }
    }))
}
