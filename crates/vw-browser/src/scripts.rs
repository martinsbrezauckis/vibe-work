//! The page scripts of the agent browser as strings ready for native evaluation: one request object
//! (`const REQ = {...}`) and the shared helpers (`js/common.js`) placed in front of the script body.
//!
//! Depends on: `serde_json`, `act` (requests). Called by: `driver`, tests (the Chromium script test reads them
//! through the private `browser_scripts` binary).
//! Invariants: the request is serialised JSON (never string-concatenated page or agent text); every script is one
//! synchronous expression whose value is a JSON string.

use serde_json::{Value, json};

const COMMON: &str = include_str!("js/common.js");
const OBSERVE: &str = include_str!("js/observe.js");
const ACT: &str = include_str!("js/act.js");
const NATIVE_CLICK: &str = include_str!("js/native_click.js");
const CONSENT: &str = include_str!("js/consent.js");
const PROBE: &str = include_str!("js/probe.js");
const ANNOTATE: &str = include_str!("js/annotate.js");
const RECHECK: &str = include_str!("js/recheck.js");
const SECRET_CAPTURE: &str = include_str!("js/secret_capture.js");
const FILL: &str = include_str!("js/fill.js");
const DESIGN: &str = include_str!("js/design.js");
const STRUCTURE: &str = include_str!("js/structure.js");
const MEDIA: &str = include_str!("js/media.js");
const PAGE: &str = include_str!("js/page.js");
const FULLPAGE: &str = include_str!("js/fullpage.js");

fn assemble(body: &str, request: &Value) -> String {
    let prelude = format!("const REQ = {request};\n{COMMON}");
    body.replace("__VW_COMMON__", &prelude)
}

/// The observe script for the page environment `page`.
#[must_use]
pub fn observe(page: &str, max_refs: usize, max_text: usize, text_offset: usize) -> String {
    observe_focused(page, max_refs, max_text, text_offset, None)
}

/// Observes matching controls before the ref cap, using only their redacted display fields.
#[must_use]
pub fn observe_focused(
    page: &str,
    max_refs: usize,
    max_text: usize,
    text_offset: usize,
    find: Option<&str>,
) -> String {
    assemble(
        OBSERVE,
        &json!({"page": page, "maxRefs": max_refs, "maxText": max_text, "textOffset": text_offset, "find": find}),
    )
}

/// The act script for one page action (`request` = the act request with `page` set).
#[must_use]
pub fn act(request: &Value) -> String {
    assemble(ACT, request).replace("__VW_NATIVE_CLICK__", NATIVE_CLICK)
}

/// Final boolean validation, evaluated in the OS engine's UI callback. Repeats authority, hit test and geometry;
/// no DOM events are sent. Changed document, target, point or viewport refuses before native input.
#[must_use]
pub fn native_validation(request: &Value, point: vw_preview::NativeClickPoint) -> String {
    let mut request = request.clone();
    request["nativeValidateOnly"] = json!(true);
    let script = act(&request);
    let script = script.trim().trim_end_matches(';');
    let expected = serde_json::to_string(&point).unwrap_or_else(|_| "null".to_owned());
    format!(
        "(() => {{ /* RC1_NATIVE_VALIDATION */ const answer = JSON.parse({script}); const expected = {expected}; return answer.status === 'done' && Boolean(answer.nativePoint) && expected && Object.keys(expected).every(key => Math.abs(answer.nativePoint[key] - expected[key]) < 0.01); }})()"
    )
}

/// Reads the current authorization document and its scope/client fingerprint.
#[must_use]
pub fn consent(page: &str, reference: Option<&str>) -> String {
    assemble(CONSENT, &json!({"page": page, "ref": reference}))
}

/// The probe script (`text`: is it on the page; `reference`: is the ref there and visible; `dialogs`: drain the
/// dialogs answered during an agent action).
#[must_use]
pub fn probe(page: &str, text: Option<&str>, reference: Option<&str>, dialogs: bool) -> String {
    assemble(
        PROBE,
        &json!({"page": page, "text": text, "ref": reference, "dialogs": dialogs}),
    )
}

/// The annotate script (`request`: `{page, name, op: start|poll|cancel, mode: element|box}`).
#[must_use]
pub fn annotate(request: &Value) -> String {
    assemble(ANNOTATE, request)
}

/// The recheck script (`request`: `{page, items: [{ref?, text}]}`).
#[must_use]
pub fn recheck(request: &Value) -> String {
    assemble(RECHECK, request)
}

/// The design-system capture script (`max_elements` elements measured at most).
#[must_use]
pub fn design(page: &str, max_elements: usize) -> String {
    assemble(DESIGN, &json!({"page": page, "maxElements": max_elements}))
}

/// The structure capture script.
#[must_use]
pub fn structure(page: &str, max_elements: usize) -> String {
    assemble(
        STRUCTURE,
        &json!({"page": page, "maxElements": max_elements}),
    )
}

/// The media listing script.
#[must_use]
pub fn media(page: &str, max_items: usize) -> String {
    assemble(MEDIA, &json!({"page": page, "maxItems": max_items}))
}

/// The whole-page text script: `request` = `{op, format, token, offset, maxChars, chunk}`.
#[must_use]
pub fn page_text(page: &str, request: &Value) -> String {
    let mut request = request.clone();
    request["page"] = json!(page);
    assemble(PAGE, &request).replace("__VW_SINGLE_HTML__", include_str!("js/single_html.js"))
}

/// The full-page screenshot helper script: `op` = metrics, scroll (to `y`), eager, pin, unpin.
#[must_use]
pub fn fullpage(page: &str, op: &str, y: f64) -> String {
    assemble(FULLPAGE, &json!({"page": page, "op": op, "y": y}))
}

/// Full-page helper bound to one capture's temporary state in the original document.
pub(crate) fn fullpage_capture(page: &str, op: &str, y: f64, token: &str) -> String {
    assemble(
        FULLPAGE,
        &json!({"page": page, "op": op, "y": y, "capture": token}),
    )
}

/// The user's selection or element picker for a vault save.
#[must_use]
pub fn secret_choice(page: &str, mode: &str, op: &str) -> String {
    assemble(
        SECRET_CAPTURE,
        &json!({"page": page, "mode": mode, "op": op}),
    )
}

/// The vault fill script: `request` = `{page, ref, role, origin, check?, value?}`. With a value the returned
/// script holds it: callers keep it in a zeroizing buffer and never log it.
#[must_use]
pub fn fill(request: &Value) -> String {
    assemble(FILL, request)
}

/// Visible labels and destinations for Linux PDF link annotations.
#[must_use]
pub fn pdf_links(page: &str) -> String {
    let request = json!({"page": page});
    assemble(include_str!("js/pdf_links.js"), &request)
}
