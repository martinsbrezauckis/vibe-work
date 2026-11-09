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
    for (key, schema) in [
        (
            "windowId",
            json!({"type":"string","maxLength":256,"description":"window.focus/close: exact id from this chat's window.open receipt or list_tabs."}),
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
