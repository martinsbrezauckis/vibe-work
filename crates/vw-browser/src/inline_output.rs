//! Whole-result byte and indentation budgets for browser reads and receipts. Called through `observe::fit`.
//! Payloads and error details shrink; outcome flags, guard verdicts, snapshots and retained ref identities stay exact.

use serde_json::{Value, json};

use crate::observe::{MAX_BUDGET, MIN_BUDGET, PRETTY_BUDGET, size};

fn fits(out: &Value, budget: usize) -> bool {
    size(out) <= budget
        && serde_json::to_string_pretty(out).is_ok_and(|text| text.len() <= PRETTY_BUDGET)
}

fn measure(out: &mut Value) {
    if let Some(content) = out.get_mut("untrustedPageContent") {
        crate::text_paging::update(content);
        crate::choices::update(content);
    }
    if out.get("trimmed").is_some() {
        out["trimmed"]["how"] = json!(continuation(out));
    }
    // Include the size field itself; decimal width reaches a fixed point after at most a few passes.
    for _ in 0..4 {
        let bytes = size(out);
        let measured = json!({"bytes":bytes,"approxTokens":bytes.div_ceil(4)});
        if out["size"] == measured {
            break;
        }
        out["size"] = measured;
    }
}

fn continuation(out: &Value) -> String {
    if out["action"] == "list_tabs" {
        return if out["nextTab"].is_string() {
            "Tab list shortened. Continue browser_act with action:\"list_tabs\",tab:nextTab to list the following tabs."
        } else {
            "Tab details shortened. browser_read {tab:\"id\"} reads an accessible tab; use its id from tabs."
        }.into();
    }
    let reservation =
        if out["refsTotal"].as_u64().unwrap_or(0) > out["refsShown"].as_u64().unwrap_or(0) {
            " At least half the ref slots reserve the main document."
        } else {
            ""
        };
    if out["untrustedPageContent"]["coveringFrames"]
        .as_array()
        .is_some_and(|frames| {
            frames
                .iter()
                .any(|frame| frame["nextChoiceCursor"].is_object())
        })
    {
        let recovery = if out.get("Outcome").is_some() {
            " Do not replay that action."
        } else {
            ""
        };
        return format!(
            "Output shortened.{reservation} Continue browser_read: choiceCursor from the frame's nextChoiceCursor for choices, or find for a label. Keep tab.{recovery}"
        );
    }
    let page = out["untrustedPageContent"]["textPageNumber"]
        .as_u64()
        .unwrap_or(0);
    let paging = if out["untrustedPageContent"]["textCursor"].is_object() {
        "textCursor:nextTextCursor, maxRefs:0".to_owned()
    } else {
        format!("textPage:{page},find:\"words\"")
    };
    let action_recovery = if out.get("Outcome").is_some() {
        " Do not replay that action."
    } else {
        ""
    };
    format!(
        "Output shortened.{reservation} Continue browser_read: find for controls; {paging} for text. Keep tab.{action_recovery}"
    )
}

fn record(out: &mut Value, field: &str, count: usize) {
    let earlier = out["trimmed"][field].as_u64().unwrap_or(0);
    if !out["trimmed"].is_object() {
        out["trimmed"] = json!({"refs":0,"textChars":0,"otherItems":0,"how":continuation(out)});
    }
    out["trimmed"][field] = json!(earlier + count as u64);
    out["trimmed"]["how"] = json!(continuation(out));
    // The same recovery instruction covers the scan cap and the byte cap, once.
    if let Some(map) = out.as_object_mut() {
        map.remove("note2");
    }
}

fn trim_text(value: &mut Value, minimum: usize) -> Option<usize> {
    if let Some(lines) = value.as_array_mut() {
        if lines.len() > 1 {
            return lines
                .pop()
                .map(|line| line.as_str().map_or(0, |s| s.chars().count()));
        }
        return lines.first_mut().and_then(|line| trim_text(line, minimum));
    }
    let text = value.as_str()?;
    let length = text.chars().count();
    if length <= minimum {
        return None;
    }
    let keep = (length / 2).max(minimum);
    *value = json!(text.chars().take(keep).collect::<String>());
    Some(length - keep)
}

fn trim_refs(content: &mut Value, minimum: usize) -> bool {
    for key in ["refs", "newRefs", "updatedRefs"] {
        if let Some(refs) = content.get_mut(key).and_then(Value::as_array_mut)
            && refs.len() > minimum
        {
            let main = refs.iter().filter(|r| r["frame"] != true).count();
            let reserve = main.min((refs.len() - 1).div_ceil(2));
            let index = refs
                .iter()
                .rposition(|r| r["frame"] == true || main > reserve);
            if let Some(index) = index {
                refs.remove(index);
            }
            return true;
        }
    }
    false
}

fn trim_links(content: &mut Value) -> bool {
    for key in ["refs", "newRefs", "updatedRefs"] {
        if let Some(refs) = content.get_mut(key).and_then(Value::as_array_mut)
            && refs.len() > 8
        {
            let main = refs.iter().filter(|r| r["frame"] != true).count();
            let reserve = main.min((refs.len() - 1).div_ceil(2));
            if let Some(index) = refs.iter().rposition(|item| {
                item["role"] == "link" && (item["frame"] == true || main > reserve)
            }) {
                refs.remove(index);
                return true;
            }
        }
    }
    false
}

fn trim_extras(content: &mut Value) -> bool {
    for path in [
        "/consoleErrorsSinceAction",
        "/frames",
        "/dialogsAnswered",
        "/refusedNavigations",
        "/changed/removedText",
        "/changed/addedText",
    ] {
        if let Some(items) = content.pointer_mut(path).and_then(Value::as_array_mut)
            && !items.is_empty()
        {
            items.pop();
            return true;
        }
    }
    if content
        .get_mut("mainHeadings")
        .and_then(Value::as_array_mut)
        .is_some_and(|headings| headings.pop().is_some())
    {
        content["mainHeadingsOmitted"] =
            json!(content["mainHeadingsOmitted"].as_u64().unwrap_or(0) + 1);
        return true;
    }
    false
}

// A single select's options or a long title/URL can outweigh the controls. Reduce those before refs.
// Never shorten addresses into different usable addresses, and never touch ref/document/frame tokens.
fn trim_detail(value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            for key in ["url", "href", "src"] {
                if map.get(key).is_some_and(|v| size(v) > 160) {
                    map.remove(key);
                    map.insert(format!("{key}Omitted"), json!(true));
                    return true;
                }
            }
            if let Some(options) = map.get_mut("options").and_then(Value::as_array_mut)
                && !options.is_empty()
            {
                options.pop();
                map.insert("optionsOmitted".into(), json!(true));
                return true;
            }
            for key in [
                "name",
                "title",
                "value",
                "valueNow",
                "outcomeText",
                "from",
                "to",
            ] {
                if let Some(text) = map.get_mut(key)
                    && trim_text(text, 60).is_some()
                {
                    return true;
                }
            }
            // Choice names are an inventory, not a preview: keep them exact or mark omissions.
            map.iter_mut()
                .filter(|(key, _)| !matches!(key.as_str(), "coveringFrames" | "choiceContext"))
                .any(|(_, value)| trim_detail(value))
        }
        Value::Array(items) => items.iter_mut().any(trim_detail),
        _ => false,
    }
}

fn trim_tabs(out: &mut Value) -> bool {
    if out["action"] != "list_tabs" {
        return false;
    }
    let Some(tabs) = out.get_mut("tabs").and_then(Value::as_array_mut) else {
        return false;
    };
    if tabs.len() <= 1 {
        return false;
    }
    tabs.pop();
    let next = tabs.last().map(|tab| tab["id"].clone());
    if let Some(content) = out["untrustedPageContent"]
        .get_mut("tabs")
        .and_then(Value::as_array_mut)
    {
        content.pop();
    }
    out["nextTab"] = next.unwrap_or(Value::Null);
    true
}

fn trim_upload(out: &mut Value) -> bool {
    if out["result"]["action"] != "upload" {
        return false;
    }
    let Some(files) = out["result"].get_mut("files").and_then(Value::as_array_mut) else {
        return false;
    };
    for file in files.iter_mut().rev() {
        if let Some(fields) = file.as_object_mut()
            && fields.remove("name").is_some()
        {
            fields.insert("nameOmitted".into(), json!(true));
            out["result"]["fileNamesOmitted"] = json!(true);
            return true;
        }
    }
    false
}

pub(crate) fn fit(out: &mut Value, budget: usize) {
    let budget = budget.clamp(MIN_BUDGET, MAX_BUDGET);
    // Images leave the text channel as MCP image blocks. They must remain byte-identical.
    let image = out.as_object_mut().and_then(|map| map.remove("vwImage"));
    measure(out);
    while !fits(out, budget) {
        if let Some(warning) = out.get_mut("warning")
            && warning.is_string()
            && !fits(warning, budget / 2)
        {
            fit_message(warning, budget / 2);
            measure(out);
            continue;
        }
        if trim_upload(out) {
            record(out, "otherItems", 1);
            measure(out);
            continue;
        }
        if out["action"] == "list_tabs"
            && (trim_detail(&mut out["untrustedPageContent"]) || trim_tabs(out))
        {
            record(out, "otherItems", 1);
            measure(out);
            continue;
        }
        let content = &mut out["untrustedPageContent"];
        let (field, count) = if crate::choices::bound(content, budget / 2) {
            ("otherItems", 1)
        } else if trim_links(content) {
            ("refs", 1)
        } else if let Some(cut) = content
            .get_mut("text")
            .and_then(|text| trim_text(text, 200))
        {
            ("textChars", cut)
        } else if crate::choice_context::trim(content, false)
            || trim_extras(content)
            || trim_detail(content)
            || crate::choices::trim_incomplete(content)
        {
            ("otherItems", 1)
        } else if trim_refs(content, 0) {
            ("refs", 1)
        } else if let Some(cut) = content.get_mut("text").and_then(|text| trim_text(text, 0)) {
            ("textChars", cut)
        } else if crate::choice_context::trim(content, true) || crate::choices::bound(content, 0) {
            ("otherItems", 1)
        } else {
            break;
        };
        record(out, field, count);
        if out.get("refsShown").is_some() {
            out["refsShown"] = json!(
                out["untrustedPageContent"]["refs"]
                    .as_array()
                    .map_or(0, Vec::len)
            );
        }
        measure(out);
    }
    if let Some(image) = image {
        out["vwImage"] = image;
    }
}

/// Error text can contain a very long refused address. Keep its leading diagnosis and trailing
/// recovery instead of allowing it to bypass the result cap. Called after the final secret scrub.
pub(crate) fn fit_error(value: &mut Value) {
    fit_message(value, MAX_BUDGET);
}

fn fit_message(value: &mut Value, budget: usize) {
    if fits(value, budget) {
        return;
    }
    let Some(text) = value.as_str() else { return };
    let mut keep = 700;
    loop {
        let mut head = keep.min(text.len());
        let mut tail = text.len().saturating_sub(keep);
        while !text.is_char_boundary(head) {
            head -= 1;
        }
        while !text.is_char_boundary(tail) {
            tail += 1;
        }
        let bounded = json!(format!(
            "{}\n[error details omitted to fit inline; use browser_read to inspect the page, and do not repeat an action with an unknown outcome]\n{}",
            &text[..head],
            &text[tail..]
        ));
        if fits(&bounded, budget) {
            *value = bounded;
            return;
        }
        keep /= 2;
    }
}
