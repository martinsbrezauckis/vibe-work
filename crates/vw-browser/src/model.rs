//! The page model the observe script answers, parsed and bounded again on our side. Everything here is
//! untrusted page data: the page shares the script's JavaScript world and can change any answer, so every string
//! and list is capped here whatever the script promised, and refs must have the ref shape.
//!
//! Depends on: serde, `serde_json`. Called by: `observe`, `driver`, tests.
//! Invariants: [`parse_answer`] never panics and turns anything malformed into an error text; after
//! [`PageModel::bound`] no field exceeds its cap.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Longest answer accepted from a page script (bytes).
pub const MAX_ANSWER: usize = 2 * 1024 * 1024;
/// Refs listed in one observation at most.
pub const MAX_REFS: usize = 200;
/// Refs remembered as valid per observation at most.
pub const MAX_ALL_REFS: usize = 1000;

fn cap(text: &mut String, max: usize) {
    if text.chars().count() > max {
        *text = text.chars().take(max).collect();
    }
}

fn cap_opt(text: &mut Option<String>, max: usize) {
    if let Some(t) = text {
        cap(t, max);
    }
}

/// True for a ref of our shape (`e` + 8 lowercase hex digits).
#[must_use]
pub fn is_ref(text: &str) -> bool {
    text.len() == 9
        && text.starts_with('e')
        && text[1..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// One interactive element.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefItem {
    /// The ref.
    #[serde(rename = "ref")]
    pub id: String,
    /// Its role (`button`, `link`, `textbox`, ...).
    pub role: String,
    /// Its accessible name.
    pub name: String,
    /// Its tag.
    #[serde(default)]
    pub tag: String,
    /// The current value (never for redacted fields).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Why the value is hidden: `password`, `otp`, `card`, `vault` (a vault-filled field). Nothing else is
    /// hidden (text the user typed is readable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted: Option<String>,
    /// Input type.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Checked state (checkbox, radio, `aria-checked`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    /// `aria-expanded`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    /// `aria-pressed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressed: Option<bool>,
    /// Present (true) when disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// Present (false) when not visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    /// Present (true) when outside the viewport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offscreen: Option<bool>,
    /// Present (true) inside a same-origin frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<bool>,
    /// Required field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// A link's address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    /// A select box's options (at most 20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    /// Options beyond the 20 listed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options_more: Option<u32>,
    /// Its text, value or name looks like an API key, shown as hidden; the user can choose it for a vault save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_like: Option<bool>,
    /// The field's role in a sign-in form (`password` or `username`): `browser_act fill` may put a vault
    /// login there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// Host side only (never serialized into an answer): the origins the field's form submits to, as the
    /// observation saw them (the form-action check).
    #[serde(default, skip_serializing)]
    pub submits_to: Option<Vec<String>>,
}

/// An iframe of the page (marked, never entered when cross-origin).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameItem {
    /// Its ref.
    #[serde(rename = "ref")]
    pub id: String,
    /// Its title or name.
    #[serde(default)]
    pub name: String,
    /// Origin and path of its address.
    #[serde(default)]
    pub src: String,
    /// Same origin as the page (its elements are listed with `frame: true`).
    #[serde(default)]
    pub same_origin: bool,
    /// Shown.
    #[serde(default)]
    pub visible: bool,
}

/// The page's scroll position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scroll {
    /// Scrolled this far (px).
    #[serde(default)]
    pub y: i64,
    /// Document height (px).
    #[serde(default)]
    pub height: i64,
    /// Viewport height (px).
    #[serde(default)]
    pub viewport: i64,
}

/// What the observe script answers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageModel {
    /// The original page URL, used for navigation authorization and document comparisons.
    #[serde(default)]
    pub url: String,
    /// The URL with key-like values hidden, for agent answers; absent in older observations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_url: Option<String>,
    /// The title.
    #[serde(default)]
    pub title: String,
    /// `document.readyState`.
    #[serde(default)]
    pub ready_state: String,
    /// Interactive elements (at most the requested number).
    #[serde(default)]
    pub refs: Vec<RefItem>,
    /// Every ref of the page (valid for actions).
    #[serde(default)]
    pub all_refs: Vec<String>,
    /// How many interactive elements the page has.
    #[serde(default)]
    pub ref_count: u32,
    /// Iframes.
    #[serde(default)]
    pub frames: Vec<FrameItem>,
    /// Cross-origin frames (content not readable).
    #[serde(default)]
    pub cross_origin_frames: u32,
    /// The traversal stopped at its caps.
    #[serde(default)]
    pub truncated: bool,
    /// One window of the visible text.
    #[serde(default)]
    pub text: String,
    /// Where the window starts.
    #[serde(default)]
    pub text_offset: u64,
    /// The visible text's full length.
    #[serde(default)]
    pub text_length: u64,
    /// Hidden text (for the prompt guard only; never returned to the agent).
    #[serde(default)]
    pub hidden_text: String,
    /// Scroll position.
    #[serde(default)]
    pub scroll: Scroll,
    /// The page environment was there (typing tracker, dialog answers).
    #[serde(default)]
    pub environment: bool,
}

impl PageModel {
    /// The page URL safe to display, falling back to the original for older observations.
    #[must_use]
    pub fn displayed_url(&self) -> &str {
        self.display_url.as_deref().unwrap_or(&self.url)
    }

    /// Applies every cap (`max_text` = the requested text window).
    pub fn bound(&mut self, max_text: usize) {
        cap(&mut self.url, 2000);
        cap_opt(&mut self.display_url, 2000);
        cap(&mut self.title, 300);
        cap(&mut self.ready_state, 20);
        cap(&mut self.text, max_text);
        cap(&mut self.hidden_text, 3000);
        self.refs.retain(|r| is_ref(&r.id));
        self.refs.truncate(MAX_REFS);
        for r in &mut self.refs {
            cap(&mut r.role, 30);
            cap(&mut r.name, 160);
            cap(&mut r.tag, 20);
            cap_opt(&mut r.value, 300);
            cap_opt(&mut r.redacted, 20);
            cap_opt(&mut r.kind, 20);
            cap_opt(&mut r.href, 300);
            cap_opt(&mut r.login, 20);
            if let Some(targets) = &mut r.submits_to {
                targets.truncate(16);
                for t in targets {
                    cap(t, 200);
                }
            }
            if r.redacted.is_some() {
                r.value = None;
            }
            if let Some(options) = &mut r.options {
                options.truncate(20);
                for o in options {
                    cap(o, 80);
                }
            }
        }
        self.all_refs.retain(|r| is_ref(r));
        self.all_refs.truncate(MAX_ALL_REFS);
        self.frames.retain(|f| is_ref(&f.id));
        self.frames.truncate(20);
        for f in &mut self.frames {
            cap(&mut f.name, 160);
            cap(&mut f.src, 200);
        }
    }
}

/// Parses a page script's answer: the JSON of the returned value, which is a JSON string holding the answer object.
///
/// # Errors
/// A short reason when the answer is too large, empty (the page went away) or not the expected shape.
pub fn parse_answer<T: DeserializeOwned>(raw: &str) -> Result<T, String> {
    if raw.len() > MAX_ANSWER {
        return Err("the page's answer was too large".to_owned());
    }
    let outer: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| "the page gave no readable answer".to_owned())?;
    let inner = match outer {
        serde_json::Value::String(text) if text.is_empty() => {
            return Err("the page changed while it was read (a navigation?)".to_owned());
        }
        serde_json::Value::String(text) => serde_json::from_str(&text)
            .map_err(|_| "the page gave no readable answer".to_owned())?,
        serde_json::Value::Null => {
            return Err("the page changed while it was read (a navigation?)".to_owned());
        }
        other => other,
    };
    // serde's message can quote the page's own strings: only its category goes to the agent.
    serde_json::from_value(inner).map_err(|e| {
        format!(
            "the page's answer had an unexpected shape ({:?})",
            e.classify()
        )
    })
}
