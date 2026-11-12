//! Bounded fields for browser diagnostics and development reads, added to the common action schema.
//! Called by `browser_everyday`; action-specific validation lives in vw-browser.
use serde_json::{Value, json};

pub(crate) fn extend(properties: &mut Value) {
    let full = vw_vocabulary::label("accessModes", "auto-approve");
    for (key, schema) in [
        (
            "colorScheme",
            json!({"enum":["light","dark","system"],"description":"tabs.emulate: native page color preference on Windows/macOS; separate from size. Unsupported on WebKitGTK."}),
        ),
        (
            "requestId",
            json!({"type":"string","maxLength":128,"description":"network.headers: current request id from network.list."}),
        ),
        (
            "unredacted",
            json!({"type":"boolean","description":format!("network.headers: include raw header values, {full} only.")}),
        ),
        (
            "failedOnly",
            json!({"type":"boolean","description":"network.list: native failures or HTTP status 400 and above."}),
        ),
        (
            "method",
            json!({"type":"string","maxLength":16,"description":"network.list: HTTP method filter."}),
        ),
        (
            "status",
            json!({"type":"integer","minimum":0,"maximum":599,"description":"network.list: exact HTTP response status filter."}),
        ),
        (
            "css",
            json!({"type":"string","minLength":1,"maxLength":16000,"description":format!("page.add_style: stylesheet for the current document; {full}, with page-script vault exclusion.")}),
        ),
        (
            "expression",
            json!({"type":"string","minLength":1,"maxLength":16000,"description":format!("raw.execute: one main-world JavaScript expression, including a Promise. {full} only; excludes documents with a vault fill.")}),
        ),
        (
            "width",
            json!({"type":"integer","minimum":240,"maximum":4096,"description":"tabs.emulate: CSS viewport width; window.resize: logical content width."}),
        ),
        (
            "height",
            json!({"type":"integer","minimum":240,"maximum":4096,"description":"tabs.emulate: CSS viewport height; window.resize: logical content height."}),
        ),
        (
            "preset",
            json!({"enum":["phone","tablet","desktop"],"description":"tabs.emulate: 390×844, 768×1024 or 1280×800 CSS pixels."}),
        ),
        (
            "reset",
            json!({"type":"boolean","description":"tabs.emulate: restore the panel device-width selection and native color preference."}),
        ),
        (
            "selector",
            json!({"type":"string","minLength":1,"maxLength":1000,"description":"page.inspect: CSS selector for exactly one main-document element."}),
        ),
        (
            "attribute",
            json!({"type":"string","maxLength":128,"description":"page.inspect: optional literal attribute; credentials and field values are withheld."}),
        ),
        (
            "properties",
            json!({"type":"array","minItems":1,"maxItems":32,"items":{"type":"string","maxLength":128},"description":"page.inspect: computed CSS properties; omitted uses common layout, color and type properties."}),
        ),
        (
            "offset",
            json!({"type":"integer","minimum":0,"maximum":20000,"description":"Diagnostic records: zero-based offset after filtering."}),
        ),
        (
            "limit",
            json!({"type":"integer","minimum":1,"maximum":100,"description":"Diagnostic records per page; default 25."}),
        ),
        (
            "last",
            json!({"type":"integer","minimum":1,"maximum":100,"description":"Read the last matching diagnostic records instead of offset."}),
        ),
        (
            "level",
            json!({"enum":["log","info","debug","warn","error","exception","rejection","resource"],"description":"Filter diagnostic records by level."}),
        ),
        (
            "bypassCache",
            json!({"type":"boolean","description":"reload: bypass the HTTP cache for this navigation."}),
        ),
    ] {
        properties[key] = schema;
    }
}
