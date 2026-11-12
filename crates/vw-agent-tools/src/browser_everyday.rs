//! Shared everyday action names and bounded fields added to the built-in browser schema.
//! Called by specs; the core checks tiers and the browser driver validates action-specific arguments.
use serde_json::{Value, json};

pub(crate) fn extend(tools: &mut [Value]) {
    let Some(tool) = tools.iter_mut().find(|t| t["name"] == "browser_act") else {
        return;
    };
    let properties = &mut tool["inputSchema"]["properties"];
    properties["action"]["enum"] =
        json!(crate::browser_input::actions().keys().collect::<Vec<_>>());
    crate::browser_diagnostics::extend(properties);
    for (key, schema) in [
        (
            "format",
            json!({"type":"string","enum":["gif"],"description":"recording.start: GIF of action screenshots, without audio; at most 30 frames over two minutes."}),
        ),
        (
            "mediaId",
            json!({"type":"string","maxLength":64,"description":"Exact video/audio handle from media.read in this document."}),
        ),
        (
            "timeMs",
            json!({"type":"integer","minimum":0,"maximum":604_800_000,"description":"media.seek: position in milliseconds within the known duration."}),
        ),
        (
            "muted",
            json!({"type":"boolean","description":"media.mute: requested mute state."}),
        ),
        (
            "fields",
            json!({"type":"array","minItems":1,"maxItems":16,"description":"form: ordered field operations, checked before execution and read back individually. Stops on the first failure; earlier changes remain. Submission is separate. Total text is limited to 20000 bytes.","items":{"type":"object","properties":{"action":{"enum":["type","select","check","uncheck"]},"ref":{"type":"string"},"text":{"type":"string"},"value":{"type":"string"},"values":{"type":"array","items":{"type":"string"}},"checked":{"type":"boolean"}},"required":["action","ref"],"additionalProperties":false}}),
        ),
        (
            "windowId",
            json!({"type":"string","maxLength":256,"description":"window.focus/close/resize: exact id from this chat's window.open receipt or list_tabs."}),
        ),
        (
            "dialogId",
            json!({"type":"string","maxLength":256,"description":"Exact id returned by page.dialogs; required when answering a held native dialog."}),
        ),
        (
            "modifiers",
            json!({"type":"array","items":{"type":"string","enum":["Alt","Control","Meta","Shift"]},"maxItems":4,"uniqueItems":true,"description":"Keys held during a click or click variant."}),
        ),
        (
            "clip",
            json!({"type":"object","description":"capture.screenshot: visible rectangle in viewport CSS pixels; omit to capture the viewport, or pass ref for one element.","properties":{"x":{"type":"integer","minimum":0},"y":{"type":"integer","minimum":0},"width":{"type":"integer","minimum":1},"height":{"type":"integer","minimum":1}},"required":["x","y","width","height"],"additionalProperties":false}),
        ),
        (
            "downloadId",
            json!({"type":"string","description":"Opaque id from this chat’s download start, click receipt or files.list_downloads."}),
        ),
        (
            "area",
            json!({"type":"string","enum":["local","session"],"description":"storage.*: current site's local or session storage."}),
        ),
        (
            "keys",
            json!({"type":"array","items":{"type":"string"},"maxItems":100,"description":"storage.read: exact keys; absent values return null."}),
        ),
        (
            "operation",
            json!({"type":"string","enum":["set","remove","clear"],"description":"storage.change or cookies.change: changes only the current site."}),
        ),
        (
            "names",
            json!({"type":"array","items":{"type":"string"},"maxItems":100,"description":"cookies.read/metadata: optional names to include."}),
        ),
        (
            "name",
            json!({"type":"string","description":"cookies.change: cookie name, except clear."}),
        ),
        (
            "path",
            json!({"type":"string","description":"cookies.change: path on the current site; default /."}),
        ),
        ("secure", json!({"type":"boolean"})),
        ("httpOnly", json!({"type":"boolean"})),
        (
            "sameSite",
            json!({"type":"string","enum":["no_restriction","lax","strict","unspecified"]}),
        ),
    ] {
        properties[key] = schema;
    }
    input_fields(properties);
}

/// Pointer, wheel, wait, insertion and rich-clipboard fields, plus the shared text/key/value descriptions.
fn input_fields(properties: &mut Value) {
    properties["direction"]["enum"] = json!(["up", "down", "left", "right"]);
    properties["mime"] = json!({"type":"string","enum":["text/html","image/png"],"description":format!("Rich clipboard representation. {} required.",vw_vocabulary::label("accessModes","auto-approve"))});
    properties["base64"] = json!({"type":"string","maxLength":349_528,"description":"clipboard.write_rich: encoded PNG, up to 256 KiB decoded."});
    properties["x"] = json!({"type":"integer","minimum":-20000,"maximum":100_000,"description":"click: viewport coordinate; scroll: signed horizontal delta (-20000 to 20000)."});
    properties["y"] = json!({"type":"integer","minimum":-20000,"maximum":100_000,"description":"click: viewport coordinate; scroll: signed vertical delta (-20000 to 20000)."});
    for axis in ["deltaX", "deltaY"] {
        properties[axis] = json!({"type":"integer","minimum":-20000,"maximum":20000,"description":"wheel: native wheel delta in CSS pixels; x/y identify the observed viewport point."});
    }
    properties["selector"] = json!({"type":"string","maxLength":2000,"description":"wait: CSS selector in the main document. Use ref for an observed element, including one in a frame."});
    properties["state"] = json!({"enum":["attached","detached","visible","hidden"],"description":"wait: required state of the ref or selector; default visible."});
    properties["ms"] = json!({"type":"integer","minimum":1,"maximum":10000,"description":"wait: plain delay, without a condition."});
    properties["load"] = json!({"oneOf":[{"type":"boolean"},{"enum":["interactive","complete"]}],"description":"wait: DOM readiness; true means the page finished loading."});
    properties["clear"] = json!({"type":"boolean","description":"type: false inserts at the current selection without clearing the field or sending key events; default true replaces its contents. highlight: true removes the temporary target outline."});
    properties["text"]["description"] = json!(
        "type: replacement text; wait: text to wait for; page.dialog.prompt: prompt answer up to 60000 bytes; clipboard.write: plain text up to 65536 UTF-8 bytes, empty clears it."
    );
    properties["key"]["description"] = json!(
        "press: key or chord (Control+a, Shift+Tab). storage.change: exact key (not needed for clear)."
    );
    properties["value"]["description"] = json!(
        "select: option value or text. storage.change set: text up to 60000 bytes; cookies.change set: text up to 4096 bytes."
    );
}

/// Each step has the exact single-action schema, without recursive batches.
pub(crate) fn batches(tools: &mut [Value]) {
    let Some(tool) = tools.iter_mut().find(|t| t["name"] == "browser_act") else {
        return;
    };
    let single = tool["inputSchema"].clone();
    tool["inputSchema"]["properties"]["steps"] = json!({"type":"array","minItems":1,"maxItems":16,
        "items":single,"description":"Ordered actions, each with its own fields and permissions. Stops at the first refusal, unmet condition, unknown outcome or budget limit; returns each completed step's actual receipt. Omit all other top-level fields."});
    if let Some(schema) = tool["inputSchema"].as_object_mut() {
        schema.remove("required");
    }
    tool["inputSchema"]["oneOf"] = json!([
        {"required":["action"],"not":{"required":["steps"]}},
        // A batch carries only its steps, plus the transport receipt Vibe's hook adds after approval.
        {"required":["steps"],"propertyNames":{"enum":["steps","_vwReceipt"]}}
    ]);
}
