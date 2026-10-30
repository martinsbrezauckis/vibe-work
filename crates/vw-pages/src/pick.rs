//! "Point Vibe at this" and page comments: the script the ENGINE evaluates in a page to find what the user means
//! (the selected text, or the element they click next), and the parsing of its answer. Adapted from ShellX
//! `shellx_browser_element_identity.rs:1-60` @ e38c2b5c (DOM path segments `tag#id`, `[data-testid]`, `[name]`,
//! `:nth-of-type`, depth 16, clip 512; role by tag); ShellX's fingerprint hashes and shadow/frame paths are left out.
//!
//! Depends on: serde, getrandom. Called by: `service` (pages), vw-preview users through vw-core-api (preview
//! elements), tests.
//! Invariants:
//! - The answer is untrusted page data: parsed as JSON only, every field length-capped again here (path 512,
//!   ids 8 × 100, text and quote 2,000, url 512, title 200); a malformed answer is "nothing picked", never an error the page
//!   can use.
//! - With a non-empty selection the pick answers at once; otherwise the next click (capture phase, default action
//!   prevented) answers, Escape cancels. The overlay is removed when it answers.
//! - For pages, the caller never trusts `ids`: it takes the first one that exists in the page FILE (`page.rs`).

use serde::{Deserialize, Serialize};

/// Longest answer parsed (bytes).
const MAX_ANSWER: usize = 64 * 1024;

/// What the user pointed at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Picked {
    /// A readable DOM path (`main > section#steps > li:nth-of-type(2)`).
    pub path: String,
    /// `id` and `data-step` values of the element and its ancestors, nearest first (at most 8).
    pub ids: Vec<String>,
    /// The element's tag.
    pub tag: String,
    /// Its role (`link`, `button`, `heading`, the tag otherwise).
    pub role: String,
    /// Its text (at most 2,000 characters).
    pub text: String,
    /// The selected text, when the pick came from a selection.
    pub quote: Option<String>,
    /// The page URL.
    pub url: String,
    /// The page's title (at most 200 characters), for the chip's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub title: Option<String>,
}

/// The state of a pick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickAnswer {
    /// Still waiting for a click.
    Pending,
    /// Escape, or the page went away.
    Cancelled,
    /// Done.
    Picked(Picked),
}

/// A random accessor name for one pick (`__vw_pick_<16 hex>`; a collision guard, not a secret).
#[must_use]
pub fn accessor_name() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        return "__vw_pick_fallback".to_owned();
    }
    format!("__vw_pick_{}", crate::comments::hex(&bytes))
}

fn quoted(name: &str) -> String {
    serde_json::Value::String(name.to_owned()).to_string()
}

/// The page-side helpers both the click pick and the named-target pick use: `clip`, the readable DOM `path`, the
/// element `role` and `describe(el, quote)`, which builds the answer object the host parses.
pub(crate) const DESCRIBE_JS: &str = r##"  const clip = (v, n) => String(v || "").replace(/\s+/g, " ").trim().slice(0, n);
  const esc = (v) => (window.CSS && CSS.escape) ? CSS.escape(String(v)) : String(v).replace(/[^a-zA-Z0-9_-]/g, "\\$&");
  const role = (el) => {
    const r = el.getAttribute && el.getAttribute("role"); if (r) return clip(r, 40);
    const t = String(el.localName || "").toLowerCase();
    if (t === "a") return "link"; if (t === "button") return "button";
    if (/^h[1-6]$/.test(t)) return "heading"; if (t === "li") return "listitem";
    return t || "element";
  };
  const seg = (el) => {
    const t = el.localName.toLowerCase();
    if (el.id) return t + "#" + esc(el.id);
    const s = el.getAttribute("data-step"); if (s) return t + '[data-step="' + clip(s, 60) + '"]';
    const n = el.getAttribute("name"); if (n) return t + '[name="' + clip(n, 60) + '"]';
    const p = el.parentElement; if (!p) return t;
    const same = Array.from(p.children).filter((c) => c.localName === el.localName);
    return same.length > 1 ? t + ":nth-of-type(" + (same.indexOf(el) + 1) + ")" : t;
  };
  const path = (el) => {
    const parts = []; let n = el, d = 0;
    while (n && n.nodeType === 1 && d < 16 && n.localName !== "html") { parts.unshift(seg(n)); n = n.parentElement; d++; }
    return clip(parts.join(" > "), 512);
  };
  const describe = (el, quote) => {
    const ids = [];
    for (let n = el; n && n.nodeType === 1 && ids.length < 8; n = n.parentElement) {
      const id = n.getAttribute("id"); if (id) ids.push(clip(id, 100));
      const st = n.getAttribute("data-step"); if (st && ids.length < 8) ids.push(clip(st, 100));
    }
    return { state: "picked", path: path(el), ids, tag: el.localName, role: role(el),
      text: clip(el.innerText, 2000), quote: quote ? clip(quote, 2000) : null,
      url: String(location.href).slice(0, 512), title: clip(document.title, 200) };
  };"##;

/// The script that starts a pick (its value is the first answer).
#[must_use]
pub fn start_script(name: &str) -> String {
    format!(
        r#"(() => {{
  const NAME = {name};
  if(document.querySelector('iframe[data-hb-prototype]') && typeof window.__hbInline==='function') {{
    window.__hbInline('pick',NAME);
    Object.defineProperty(window,NAME,{{value:()=>window.__hbInline('pick-poll'),configurable:true}});
    Object.defineProperty(window,NAME+'_stop',{{value:()=>window.__hbInline('pick-cancel'),configurable:true}});
    return JSON.stringify({{state:'pending'}});
  }}
{DESCRIBE_JS}
  const old = window[NAME + "_stop"]; if (typeof old === "function") old();
  const st = {{ result: {{ state: "pending" }} }};
  const sel = window.getSelection && window.getSelection();
  if (sel && !sel.isCollapsed && String(sel).trim()) {{
    let node = sel.getRangeAt(0).commonAncestorContainer;
    if (node.nodeType !== 1) node = node.parentElement;
    st.result = describe(node || document.body, String(sel));
  }} else {{
    const box = document.createElement("div");
    box.style.cssText = "position:fixed;pointer-events:none;z-index:2147483647;outline:2px solid #fa500f;background:#fa500f1a;border-radius:3px;display:none";
    document.documentElement.appendChild(box);
    const move = (e) => {{ const r = e.target.getBoundingClientRect && e.target.getBoundingClientRect(); if (!r) return;
      box.style.display = "block"; box.style.left = r.left + "px"; box.style.top = r.top + "px";
      box.style.width = r.width + "px"; box.style.height = r.height + "px"; }};
    const stop = () => {{ window.removeEventListener("mousemove", move, true); window.removeEventListener("click", click, true);
      window.removeEventListener("keydown", key, true); box.remove(); }};
    const click = (e) => {{ e.preventDefault(); e.stopImmediatePropagation(); st.result = describe(e.target, ""); stop(); }};
    const key = (e) => {{ if (e.key === "Escape") {{ e.preventDefault(); st.result = {{ state: "cancelled" }}; stop(); }} }};
    window.addEventListener("mousemove", move, true); window.addEventListener("click", click, true);
    window.addEventListener("keydown", key, true);
    Object.defineProperty(window, NAME + "_stop", {{ value: () => {{ st.result = {{ state: "cancelled" }}; stop(); }}, configurable: true }});
  }}
  Object.defineProperty(window, NAME, {{ value: () => JSON.stringify(st.result), configurable: true }});
  return JSON.stringify(st.result);
}})()"#,
        name = quoted(name)
    )
}

/// The script that reads a pick's state.
#[must_use]
pub fn poll_script(name: &str) -> String {
    format!(
        "(() => {{ const f = window[{n}]; return typeof f === 'function' ? f() : '{{\"state\":\"gone\"}}'; }})()",
        n = quoted(name)
    )
}

/// The script that cancels a pick (removes the overlay).
#[must_use]
pub fn cancel_script(name: &str) -> String {
    format!(
        "(() => {{ const f = window[{n} + '_stop']; if (typeof f === 'function') f(); return 'null'; }})()",
        n = quoted(name)
    )
}

/// Where a pick points when no person clicks (a Debug API request names the element).
#[derive(Clone, Debug, PartialEq)]
pub enum PickAt {
    /// The first element a CSS selector matches.
    Selector(String),
    /// The element at a point of the page's viewport (CSS pixels).
    Point {
        /// Distance from the left edge.
        x: f64,
        /// Distance from the top edge.
        y: f64,
    },
}

/// Why a named target gave no element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetMiss {
    /// Nothing matches the selector, or nothing is at the point.
    NotFound,
    /// The selector is not a valid CSS selector.
    InvalidSelector,
    /// The page did not answer in the expected form.
    Unreadable,
}

/// The script that picks `at` at once: no overlay and no waiting. Its answer has the shape of a click pick
/// (`describe` is the same function), so [`parse_target`] returns the same [`Picked`] a person would have caused.
#[must_use]
pub fn target_script(at: &PickAt) -> String {
    target_script_with_quote(at, None)
}

/// A named target with an optional exact selection; a selection absent from the element is refused.
#[must_use]
pub fn target_script_with_quote(at: &PickAt, quote: Option<&str>) -> String {
    let at = match at {
        PickAt::Selector(selector) => serde_json::json!({ "selector": selector }),
        PickAt::Point { x, y } => serde_json::json!({ "x": x, "y": y }),
    };
    let quote = serde_json::json!(quote);
    format!(
        r#"(() => {{
  const AT = {at};
{DESCRIBE_JS}
  let el = null;
  try {{
    el = AT.selector !== undefined ? document.querySelector(AT.selector) : document.elementFromPoint(AT.x, AT.y);
  }} catch (e) {{ return JSON.stringify({{ state: "invalid" }}); }}
  if (!el || el.nodeType !== 1) return JSON.stringify({{ state: "notfound" }});
  const quote = {quote};
  if (quote !== null && !String(el.innerText || el.textContent || "").includes(quote)) return JSON.stringify({{ state: "notfound" }});
  return JSON.stringify(describe(el, quote || ""));
}})()"#
    )
}

/// Reads the same named target for Copy, preserving line breaks up to the menu's copy bound.
#[must_use]
pub fn copy_target_script(at: &PickAt, quote: Option<&str>) -> String {
    target_script_with_quote(at, quote).replace(
        "return JSON.stringify(describe(el, quote || \"\"));",
        "return JSON.stringify({ state: 'copied', text: String(quote ?? el.innerText ?? el.textContent ?? '').slice(0, 100000) });",
    )
}

/// Parses a named Copy result, refusing missing targets and bounding untrusted page text again.
#[must_use]
pub fn parse_named_copy(raw: &str) -> Option<String> {
    if raw.len() > 800_000 {
        return None;
    }
    let outer: serde_json::Value = serde_json::from_str(raw).ok()?;
    let value = match outer {
        serde_json::Value::String(inner) => serde_json::from_str(&inner).ok()?,
        other => other,
    };
    if value.get("state")?.as_str()? != "copied" {
        return None;
    }
    Some(value.get("text")?.as_str()?.chars().take(100_000).collect())
}

/// Parses the answer of [`target_script`].
///
/// # Errors
/// [`TargetMiss`] when the selector matched nothing, was invalid, or the answer was not readable.
pub fn parse_target(result: &str) -> Result<Picked, TargetMiss> {
    if let PickAnswer::Picked(picked) = parse(result) {
        return Ok(picked);
    }
    Err(match state_of(result).as_deref() {
        Some("notfound") => TargetMiss::NotFound,
        Some("invalid") => TargetMiss::InvalidSelector,
        _ => TargetMiss::Unreadable,
    })
}

/// The `state` field of an evaluation result (the JSON of a returned JSON string), when readable.
fn state_of(result: &str) -> Option<String> {
    if result.len() > MAX_ANSWER {
        return None;
    }
    let outer: serde_json::Value = serde_json::from_str(result).ok()?;
    let value = match outer {
        serde_json::Value::String(inner) => serde_json::from_str(&inner).unwrap_or_default(),
        other => other,
    };
    value
        .get("state")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn text(value: Option<&serde_json::Value>, max: usize) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.chars().take(max).collect(),
        _ => String::new(),
    }
}

/// Parses an evaluation result (the JSON of the returned value: a JSON string holding the answer object).
#[must_use]
pub fn parse(result: &str) -> PickAnswer {
    if result.len() > MAX_ANSWER {
        return PickAnswer::Cancelled;
    }
    let outer: serde_json::Value = match serde_json::from_str(result) {
        Ok(v) => v,
        Err(_) => return PickAnswer::Cancelled,
    };
    let value = match outer {
        serde_json::Value::String(inner) => serde_json::from_str(&inner).unwrap_or_default(),
        other => other,
    };
    match value.get("state").and_then(serde_json::Value::as_str) {
        Some("pending") => PickAnswer::Pending,
        Some("picked") => {
            let ids = value
                .get("ids")
                .and_then(serde_json::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(|s| s.chars().take(100).collect::<String>())
                        .filter(|s| !s.is_empty())
                        .take(8)
                        .collect()
                })
                .unwrap_or_default();
            let quote = text(value.get("quote"), 2000);
            PickAnswer::Picked(Picked {
                path: text(value.get("path"), 512),
                ids,
                tag: text(value.get("tag"), 40),
                role: text(value.get("role"), 40),
                text: text(value.get("text"), 2000),
                quote: Some(quote).filter(|q| !q.is_empty()),
                url: text(value.get("url"), 512),
                title: Some(text(value.get("title"), 200)).filter(|t| !t.is_empty()),
            })
        }
        _ => PickAnswer::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::{PickAnswer, parse};

    #[test]
    fn a_pick_carries_the_page_title_bounded_and_none_when_empty() {
        let answer = |title: &str| {
            serde_json::Value::String(
                serde_json::json!({"state": "picked", "path": "main > p", "ids": [], "tag": "p",
                    "role": "p", "text": "Hello", "quote": "Hello", "url": "http://127.0.0.1:5173/",
                    "title": title})
                .to_string(),
            )
            .to_string()
        };
        let PickAnswer::Picked(picked) = parse(&answer("Shop")) else {
            panic!("not picked")
        };
        assert_eq!(picked.title.as_deref(), Some("Shop"));
        let PickAnswer::Picked(long) = parse(&answer(&"t".repeat(500))) else {
            panic!("not picked")
        };
        assert_eq!(long.title.map(|t| t.chars().count()), Some(200));
        let PickAnswer::Picked(none) = parse(&answer("")) else {
            panic!("not picked")
        };
        assert_eq!(none.title, None);
    }
}
