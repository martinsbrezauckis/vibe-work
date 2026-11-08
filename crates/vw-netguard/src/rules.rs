//! Browser content rules preserve URL credential refusal without restricting local addresses.
//! Called by native webviews; generation tracking prevents stale asynchronous rule installs.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};

/// Whose rights a webview's rule list applies (a mount keeps one for its life; Local is regenerated when the ports
/// change).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RulePage {
    /// A page from the web.
    Web,
    /// A page of the session's own server: these loopback ports (Vibe Work's own ports never among them).
    Local {
        /// Allowed ports, any order.
        ports: Vec<u16>,
    },
    /// The local-network page the user opened on `host` (lowercase; IPv6 in brackets).
    Lan {
        /// The page's host as it appears in its URL.
        host: String,
    },
}

/// The browser's content rules do not distinguish public and local destinations.
#[must_use]
pub fn content_rules(_page: &RulePage) -> Value {
    json!([{"trigger":{"url-filter":"^[a-z]+://[^/]*@"},"action":{"type":"block"}}])
}

/// [`content_rules`] as the JSON text `WKContentRuleListStore` compiles.
#[must_use]
pub fn content_rules_json(page: &RulePage) -> String {
    content_rules(page).to_string()
}

/// Orders rule-list updates per webview (an engine compiles lists asynchronously, so a compile that
/// finishes after a newer one was started must not install its older list). `begin` before starting a compile;
/// install only while `is_latest` holds for its generation.
#[derive(Debug, Default)]
pub struct RuleGenerations {
    latest: Mutex<HashMap<String, u64>>,
}

impl RuleGenerations {
    /// Starts a new update for `key` (a webview label) and returns its generation.
    pub fn begin(&self, key: &str) -> u64 {
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        let next = latest.get(key).map_or(1, |g| g + 1);
        latest.insert(key.to_owned(), next);
        next
    }

    /// True while `generation` is the newest update started for `key`.
    #[must_use]
    pub fn is_latest(&self, key: &str, generation: u64) -> bool {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .is_some_and(|g| *g == generation)
    }

    /// Forgets `key` (its webview closed): no update of it installs any more.
    pub fn forget(&self, key: &str) {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }
}
