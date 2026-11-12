//! Observation byte budgets and capture trimming. Depends on JSON and the parent text cap; called by read/receipt formatting.
use super::chars;
use serde_json::{Value, json};

/// Bytes of `value` as compact JSON.
#[must_use]
pub fn size(value: &Value) -> usize {
    serde_json::to_string(value).map_or(0, |t| t.len())
}

/// Shrinks a preview or focused text list, preserving its first paragraph until the last resort.
fn trim_text(value: &mut Value, minimum: usize) -> Option<usize> {
    let item = if let Some(lines) = value.as_array_mut() {
        if lines.len() > 1 {
            return lines
                .pop()
                .and_then(|line| line.as_str().map(|t| t.chars().count()));
        }
        lines.first_mut()?
    } else {
        value
    };
    let text = item.as_str()?;
    let length = text.chars().count();
    if length <= minimum {
        return None;
    }
    let keep = (length / 2).max(minimum);
    *item = json!(chars(text, keep));
    Some(length - keep)
}

/// Trims `out["untrustedPageContent"]` (refs from the end, then text) until `out` fits `budget` bytes; records what was
/// left out and the answer's size (`size`: bytes and approximate tokens, ~4 bytes each).
pub(crate) fn fit_capture(out: &mut Value, budget: usize) {
    // Room for the `trimmed` and `size` fields added below.
    let target = budget.saturating_sub(260);
    let (mut refs_cut, mut text_cut, mut other_cut) = (0usize, 0usize, 0usize);
    // Lists that may shrink, least useful first once refs and text are down to a minimum (a
    // receipt with console errors can exceed the budget).
    let lists = [
        "consoleErrorsSinceAction",
        "frames",
        "dialogsAnswered",
        "refusedNavigations",
        "newRefs",
    ];
    while size(out) > target {
        let content = &mut out["untrustedPageContent"];
        let refs = content["refs"].as_array().map_or(0, Vec::len);
        let changed_lines = content["changed"]["addedText"]
            .as_array()
            .map_or(0, Vec::len)
            + content["changed"]["removedText"]
                .as_array()
                .map_or(0, Vec::len);
        if refs > 8 {
            if let Some(list) = content["refs"].as_array_mut() {
                list.pop();
            }
            refs_cut += 1;
        } else if let Some(cut) = trim_text(&mut content["text"], 200) {
            text_cut += cut;
        } else if let Some(key) = lists
            .iter()
            .find(|k| content[**k].as_array().is_some_and(|l| !l.is_empty()))
        {
            if let Some(list) = content[*key].as_array_mut() {
                list.pop();
            }
            other_cut += 1;
        } else if changed_lines > 1 {
            for key in ["removedText", "addedText"] {
                if let Some(list) = content["changed"][key].as_array_mut()
                    && list.len() > usize::from(key == "addedText")
                {
                    list.pop();
                    other_cut += 1;
                    break;
                }
            }
        } else if refs > 0 {
            if let Some(list) = content["refs"].as_array_mut() {
                list.pop();
            }
            refs_cut += 1;
        } else if let Some(cut) = trim_text(&mut content["text"], 0) {
            text_cut += cut;
        } else {
            break;
        }
    }
    if refs_cut + text_cut + other_cut > 0 {
        // A second fit (the agent tool's, after it adds the tab) adds to what an earlier fit left out.
        let earlier = |key: &str| {
            out["trimmed"][key]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0)
        };
        out["trimmed"] = json!({
            "refs": refs_cut + earlier("refs"),
            "textChars": text_cut + earlier("textChars"),
            "otherItems": other_cut + earlier("otherItems"),
            "how": "ask for more with maxBytes (up to 64000), textPage, or a focused read with find",
        });
    }
    // `refsShown` counts the refs the answer still lists after trimming.
    if out.get("refsShown").is_some() {
        out["refsShown"] = json!(
            out["untrustedPageContent"]["refs"]
                .as_array()
                .map_or(0, Vec::len)
        );
    }
    let bytes = size(out);
    out["size"] = json!({"bytes": bytes, "approxTokens": bytes.div_ceil(4)});
}

/// Fits the complete read or action result, including metadata, notes and JSON indentation.
pub fn fit(out: &mut Value, budget: usize) {
    crate::inline_output::fit(out, budget);
}
