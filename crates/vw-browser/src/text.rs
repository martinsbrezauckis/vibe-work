//! Browser text normalization at model and capture boundaries. Called by the output filter and capture writer;
//! hidden format controls are removed before secret matching, while ordinary Unicode and line breaks survive.

/// Whether a character can conceal or reorder page text without drawing a character.
#[must_use]
pub fn hidden(c: char) -> bool {
    (c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        || matches!(c, '\u{00ad}' | '\u{034f}' | '\u{061c}' | '\u{180e}'
            | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{e0000}'..='\u{e007f}')
}

/// Removes hidden controls from text delivered to the agent.
#[must_use]
pub fn clean(text: &str) -> String {
    text.chars().filter(|c| !hidden(*c)).collect()
}

/// Normalizes every string in an answer before redaction. Object keys are host schema names.
pub(crate) fn clean_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            text.retain(|c| !hidden(c));
            // Normalization can join a stored value that the earlier byte scrub could not match.
            if let Some((clean, _)) = vw_secrets::scrub(text.as_bytes()) {
                *text = String::from_utf8_lossy(&clean).into_owned();
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(clean_value),
        serde_json::Value::Object(fields) => fields.values_mut().for_each(clean_value),
        _ => {}
    }
}

/// Normalizes text captures, decoding JSON escapes before filtering their strings.
pub(crate) fn capture_text(name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    if name.to_ascii_lowercase().ends_with(".json") {
        let mut value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|_| "capture JSON could not be read safely".to_owned())?;
        let before = value.clone();
        clean_value(&mut value);
        if value != before {
            return serde_json::to_vec_pretty(&value).map_err(|e| e.to_string());
        }
        return Ok(bytes.to_vec());
    }
    Ok(clean(&String::from_utf8_lossy(bytes)).into_bytes())
}
