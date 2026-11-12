//! Browser content filters: an original conservative Balanced list and ABP-list conversion for Strict.
//! Depends on adblock's parser, not page scripts; the native engine enforces the resulting `WebKit` rules.
use serde::{Deserialize, Serialize};
use serde_json::json;

/// User-selected blocking strength.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// No ad filtering. Navigation and network policy still apply.
    Off,
    /// Our small, bundled list; no downloads.
    #[default]
    Balanced,
    /// `EasyList` and `EasyPrivacy`, fetched on first selection, plus everything Balanced blocks and hides.
    Strict,
}

/// Largest downloaded list (or cached compiled rule set).
pub const MAX_LIST_BYTES: usize = 16 * 1024 * 1024;

/// Compile a complete native rule list. Strict requires downloaded ABP text.
/// Unsupported ABP constructs cannot be represented by `WebKit` content rules; scriptlets are never executed.
/// Strict ends with Balanced's own rules: a list exception (`ignore-previous-rules`) lifts only rules before it, so
/// Strict always blocks and hides at least what Balanced does.
///
/// # Errors
/// Missing, oversized or unconvertible Strict input.
pub fn compile(mode: Mode, lists: Option<&str>) -> Result<String, String> {
    match mode {
        Mode::Off => Ok("[]".into()),
        Mode::Balanced => Ok(balanced()),
        Mode::Strict => {
            let lists = lists.ok_or("Strict lists have not been downloaded")?;
            if lists.len() > MAX_LIST_BYTES * 2 {
                return Err("filter lists are too large".into());
            }
            let mut filters = adblock::lists::FilterSet::new(true);
            filters.add_filter_list(lists.to_owned(), adblock::lists::ParseOptions::default());
            let (rules, _) = filters
                .into_content_blocking()
                .map_err(|()| "filter conversion failed")?;
            if rules.is_empty() {
                return Err("filter lists contain no supported rules".into());
            }
            let mut rules = serde_json::to_value(&rules).map_err(|error| error.to_string())?;
            if let (Some(rules), serde_json::Value::Array(own)) =
                (rules.as_array_mut(), balanced_rules())
            {
                rules.extend(own);
            }
            serde_json::to_string(&rules).map_err(|error| error.to_string())
        }
    }
}

pub(crate) fn balanced() -> String {
    balanced_rules().to_string()
}

fn balanced_rules() -> serde_json::Value {
    // Original list: narrowly named advertising/tracking endpoints. Domain boundaries prevent matching
    // unrelated hosts with similar names; first-party app resources and ordinary dialogs remain intact.
    let domains = [
        "doubleclick.net",
        "googlesyndication.com",
        "googleadservices.com",
        "google-analytics.com",
        "scorecardresearch.com",
        "adnxs.com",
    ];
    let mut rules: Vec<_> = domains.iter().map(|domain| json!({
        "trigger": {"url-filter": format!("^https?://([^/]+\\.)?{}[/:]", domain.replace('.', "\\.")),
                    "load-type": ["third-party"]},
        "action": {"type": "block"}
    })).collect();
    rules.push(json!({"trigger":{"url-filter":".*"}, "action":{
        "type":"css-display-none", "selector":".adsbygoogle, [data-ad-slot], iframe[id^='google_ads_iframe_']"
    }}));
    json!(rules)
}

/// Confirmed settings and platform capability, returned by the core.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Confirmed mode.
    pub mode: Mode,
    /// Whether native content filtering is available.
    pub supported: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Saved {
    pub mode: Mode,
    pub rules: String,
}
impl Saved {
    pub fn load(root: &std::path::Path) -> Self {
        let path = root.join("browser-shields.json");
        if std::fs::metadata(&path).is_ok_and(|meta| meta.len() <= MAX_LIST_BYTES as u64)
            && let Ok(bytes) = std::fs::read(path)
            && let Ok(saved) = serde_json::from_slice::<Self>(&bytes)
            && serde_json::from_str::<Vec<serde_json::Value>>(&saved.rules).is_ok()
        {
            return saved;
        }
        Self {
            mode: Mode::Balanced,
            rules: balanced(),
        }
    }
}

/// Cross-platform evaluator for the compiled rules used by every Browser engine.
pub mod matcher;

/// Native protocol cosmetics for WebView2, testable without Windows.
pub mod cosmetic;
