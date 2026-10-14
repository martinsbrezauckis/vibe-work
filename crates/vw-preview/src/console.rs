//! Console capture for the preview: an initialization script keeps the page's recent errors and warnings in an
//! in-page ring, and the host drains it with native script evaluation (no IPC: the preview has none). New code;
//! ShellX's native engine never captured the page console; only its bounded host-side
//! log idea is adapted here.
//!
//! Depends on: serde, `serde_json`, getrandom. Called by: `service`, the Tauri engine (script text), tests.
//! Invariants:
//! - Everything the page reports is untrusted text: sizes are capped in the page AND again here (the page can
//!   replace the script's functions), entries carry no markup meaning, and the drain answer is parsed as data.
//! - In the page: at most [`PAGE_RING`] entries of at most [`MAX_TEXT`] characters; the accessor's name is random
//!   per mount and non-enumerable (not a secret: only a collision guard).

use serde::{Deserialize, Serialize};

/// Entries the page keeps between drains.
pub const PAGE_RING: usize = 200;
/// Longest message kept (characters).
pub const MAX_TEXT: usize = 2048;
/// Entries the host keeps per preview.
pub const HOST_RING: usize = 500;
/// Entries returned to the agent at most.
pub const MAX_RETURNED: usize = 50;
/// Longest drain answer parsed (bytes).
pub const MAX_DRAIN_BYTES: usize = 1024 * 1024;

/// Kind of a console entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ConsoleLevel {
    /// `console.error`.
    Error,
    /// `console.warn`.
    Warn,
    /// An uncaught exception (`error` event).
    Exception,
    /// An unhandled promise rejection.
    Rejection,
}

/// One console entry (untrusted page text).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ConsoleEntry {
    /// Kind.
    pub level: ConsoleLevel,
    /// The message (at most [`MAX_TEXT`] characters).
    pub text: String,
    /// The page URL when it was logged (at most 512 characters).
    pub url: String,
    /// Milliseconds since the Unix epoch (page clock).
    pub at_ms: u64,
}

/// A random accessor name for one mount (`__vw_console_<16 hex>`).
#[must_use]
pub fn accessor_name() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // A fixed fallback still works; the name is a collision guard, not a secret.
        return "__vw_console_fallback".to_owned();
    }
    let mut name = String::from("__vw_console_");
    for byte in bytes {
        let _ = std::fmt::Write::write_fmt(&mut name, format_args!("{byte:02x}"));
    }
    name
}

/// The initialization script for a mount (main frame only). `accessor` must come from [`accessor_name`].
#[must_use]
pub fn init_script(accessor: &str) -> String {
    format!(
        r#"(() => {{
  const NAME = {name};
  if (Object.prototype.hasOwnProperty.call(window, NAME)) return;
  const ring = [];
  const MAX = {ring}, TEXT = {text};
  const str = (v) => {{
    try {{
      if (v instanceof Error) return String(v.stack || v.message || v);
      if (typeof v === "object" && v !== null) return JSON.stringify(v);
      return String(v);
    }} catch (e) {{ return "[unprintable]"; }}
  }};
  const push = (level, parts) => {{
    try {{
      const text = parts.map(str).join(" ").slice(0, TEXT);
      ring.push({{ level, text, url: String(location.href).slice(0, 512), atMs: Date.now() }});
      if (ring.length > MAX) ring.splice(0, ring.length - MAX);
    }} catch (e) {{ /* logging must never break the page */ }}
  }};
  for (const level of ["error", "warn"]) {{
    const original = console[level];
    console[level] = function (...args) {{
      push(level, args);
      return original.apply(this, args);
    }};
  }}
  window.addEventListener("error", (e) => push("exception", [e.message || e.error || "error"]), true);
  window.addEventListener("unhandledrejection", (e) => push("rejection", [e.reason]));
  Object.defineProperty(window, NAME, {{
    value: () => JSON.stringify(ring.splice(0, ring.length)),
    enumerable: false, configurable: false, writable: false
  }});
}})();"#,
        name = serde_json::Value::String(accessor.to_owned()),
        ring = PAGE_RING,
        text = MAX_TEXT,
    )
}

/// The expression that drains the ring (evaluated natively; its JSON result is a JSON string of an array).
#[must_use]
pub fn drain_script(accessor: &str) -> String {
    format!(
        "(() => {{ const f = window[{name}]; return typeof f === 'function' ? f() : '[]'; }})()",
        name = serde_json::Value::String(accessor.to_owned())
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageEntry {
    level: Option<String>,
    text: Option<serde_json::Value>,
    url: Option<serde_json::Value>,
    at_ms: Option<f64>,
}

fn capped(value: Option<serde_json::Value>, max: usize) -> String {
    let text = match value {
        Some(serde_json::Value::String(s)) => s,
        Some(other) => other.to_string(),
        None => String::new(),
    };
    text.chars().take(max).collect()
}

/// Parses an evaluation result (`eval_with_callback` hands over the JSON of the returned value, here a JSON
/// string that itself holds the array). Malformed or oversized answers give no entries, never an error the page
/// could use to break the tool.
#[must_use]
pub fn parse_drain(result: &str) -> Vec<ConsoleEntry> {
    if result.len() > MAX_DRAIN_BYTES {
        return Vec::new();
    }
    let inner: String = match serde_json::from_str::<serde_json::Value>(result) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(serde_json::Value::Array(a)) => serde_json::Value::Array(a).to_string(),
        _ => return Vec::new(),
    };
    let Ok(entries) = serde_json::from_str::<Vec<PageEntry>>(&inner) else {
        return Vec::new();
    };
    entries
        .into_iter()
        .take(PAGE_RING)
        .filter_map(|e| {
            let level = match e.level.as_deref()? {
                "error" => ConsoleLevel::Error,
                "warn" => ConsoleLevel::Warn,
                "exception" => ConsoleLevel::Exception,
                "rejection" => ConsoleLevel::Rejection,
                _ => return None,
            };
            Some(ConsoleEntry {
                level,
                text: capped(e.text, MAX_TEXT),
                url: capped(e.url, 512),
                at_ms: e
                    .at_ms
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    // Saturating float-to-int conversion: finite and non-negative was checked above.
                    .map_or(0, |v| {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let ms = v.min(9.0e15) as u64;
                        ms
                    }),
            })
        })
        .collect()
}
