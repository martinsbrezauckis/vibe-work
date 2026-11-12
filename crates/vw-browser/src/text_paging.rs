//! Continuation within a merged text window, after final inline byte trimming.
//! Used by reads and their serializer; offsets count UTF-16 units, never bytes or control refs.
//! Depends on: the merged page model and serde.
//! Called by: browser read result fitting.
use crate::{model::PageModel, observe::PAGE_CHARS};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A position in a text page. Child-frame windows follow the main document's window.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TextCursor {
    /// The script text page (2,500 UTF-16 units per document).
    pub page: usize,
    /// UTF-16 units already returned from this merged window.
    pub offset: usize,
}

pub(crate) fn select(model: &mut PageModel, cursor: Option<&TextCursor>) {
    model.text.retain(|c| !crate::text::hidden(c));
    model.text_window_length = Some(model.text.encode_utf16().count() as u64);
    let requested = cursor.map_or(0, |cursor| cursor.offset);
    let mut skipped = 0;
    model.text = model
        .text
        .chars()
        .skip_while(|c| {
            if skipped >= requested {
                return false;
            }
            skipped += c.len_utf16();
            true
        })
        .collect();
    model.text_cursor_offset = skipped as u64;
}

pub(crate) fn include(content: &mut Value, model: &PageModel) {
    content["textCursor"] =
        json!({"page": model.text_offset / PAGE_CHARS as u64, "offset": model.text_cursor_offset});
    content["textWindowChars"] = json!(
        model
            .text_window_length
            .unwrap_or_else(|| model.text.encode_utf16().count() as u64)
    );
    update(content);
}

/// Called on every fit, including after tab metadata is attached. A cursor never jumps over trimmed text.
pub(crate) fn update(content: &mut Value) {
    let Some(text) = content["text"].as_str() else {
        return;
    };
    let Some(page) = content["textCursor"]["page"].as_u64() else {
        return;
    };
    let start = content["textCursor"]["offset"].as_u64().unwrap_or(0);
    let end = start.saturating_add(text.encode_utf16().count() as u64);
    let window = content["textWindowChars"].as_u64().unwrap_or(0);
    let next = if end < window && end > start {
        Some(json!({"page":page,"offset":end}))
    } else if end >= window && page + 1 < content["textPages"].as_u64().unwrap_or(1) {
        Some(json!({"page":page + 1,"offset":0}))
    } else {
        None
    };
    if let Some(next) = next {
        content["nextTextCursor"] = next;
    } else if let Some(map) = content.as_object_mut() {
        map.remove("nextTextCursor");
    }
    if let Some(map) = content.as_object_mut() {
        map.remove("textContinuation");
    }
}
