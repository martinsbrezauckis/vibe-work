//! Content rules for a preview webview (macOS `WKContentRuleList`): the JSON rule list that keeps a page
//! to the addresses its mount may reach, by URL, for every request the page makes (documents, frames, scripts,
//! images, `fetch`, `WebSockets`). The same address table as the egress guard ([`crate::ip::classify`] and `decide`):
//! - a page from the web reaches no address of this computer (`127.0.0.0/8`, `[::1]`, `localhost`, `*.localhost`), no
//!   private, link-local, carrier-grade NAT, multicast or reserved literal (IPv4, and every IPv6 literal outside
//!   `2000::/3`, with IPv4-mapped / NAT64 / 6to4 / Teredo forms), and no local-only name (`*.local`, `*.home.arpa`,
//!   `*.internal`);
//! - a page of the session's own server (Local) additionally reaches the given loopback ports, spelled exactly
//!   `127.0.0.1`, `localhost` or `[::1]` (the egress guard's spellings);
//! - the local-network page the user opened (Lan) additionally reaches its own host.
//!
//! The rule syntax (`WKContentRuleList`; `WebKit`'s "Introduction to `WebKit` Content Blockers"): `url-filter` is a
//! regular expression over the full URL with `.`, character sets, `?`, `*`, `+`, groups and anchors, without `|` or
//! counted repeats; rules apply in order and `ignore-previous-rules` lifts the earlier blocks for the URLs it matches.
//! URLs are matched in their canonical form (lowercase host, decimal IPv4, compressed hexadecimal IPv6), which is what
//! the patterns are written against.
//!
//! Depends on: `serde_json`. Called by: vw-preview (the rule page of a mount), vw-webview (compiles it on macOS),
//! tests.
//! Invariants:
//! - Every block rule comes before every exception; an exception names one exact host (and port, for loopback).
//! - An exception matches only at the real host: right after `scheme://` (no user info: a URL with user info is
//!   blocked for every page, sub-resources never need it, and the navigation policy refuses it too), ending at `/`
//!   or at `:` + digits + `/`. So text in the user info (`http://10.1.2.3:x@10.0.0.1/`) or a longer name
//!   (`10.1.2.3.example.com`) never lifts a block.
//! - Patterns use only the syntax above (no `|`, `{`, `\d`, lookaround).

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

/// The start of every block pattern: a scheme, `://`, optional user info.
const START: &str = "^[a-z]+://([^/@]*@)?";

/// The start of every exception: a scheme and `://`, then the host at once (never user info).
const EXCEPTION_START: &str = "^[a-z]+://";

/// Any URL with user info (an `@` before the first `/` after `://`): blocked for every page.
const USER_INFO: &str = "^[a-z]+://[^/]*@";

/// Hosts no page from the web may reach, as patterns that follow [`START`] (each ends at the host's boundary).
const BLOCKED_HOSTS: &[&str] = &[
    // This computer.
    "127\\.[0-9]+\\.[0-9]+\\.[0-9]+[:/]",
    "localhost\\.?[:/]",
    "[^/:@]*\\.localhost\\.?[:/]",
    // Private, link-local, carrier-grade NAT, unspecified.
    "10\\.[0-9]+\\.[0-9]+\\.[0-9]+[:/]",
    "172\\.1[6-9]\\.[0-9]+\\.[0-9]+[:/]",
    "172\\.2[0-9]\\.[0-9]+\\.[0-9]+[:/]",
    "172\\.3[01]\\.[0-9]+\\.[0-9]+[:/]",
    "192\\.168\\.[0-9]+\\.[0-9]+[:/]",
    "169\\.254\\.[0-9]+\\.[0-9]+[:/]",
    "100\\.6[4-9]\\.[0-9]+\\.[0-9]+[:/]",
    "100\\.[7-9][0-9]\\.[0-9]+\\.[0-9]+[:/]",
    "100\\.1[01][0-9]\\.[0-9]+\\.[0-9]+[:/]",
    "100\\.12[0-7]\\.[0-9]+\\.[0-9]+[:/]",
    "0\\.[0-9]+\\.[0-9]+\\.[0-9]+[:/]",
    // Reserved, documentation, benchmarking, multicast, broadcast.
    "192\\.0\\.0\\.[0-9]+[:/]",
    "192\\.0\\.2\\.[0-9]+[:/]",
    "198\\.1[89]\\.[0-9]+\\.[0-9]+[:/]",
    "198\\.51\\.100\\.[0-9]+[:/]",
    "203\\.0\\.113\\.[0-9]+[:/]",
    "22[4-9]\\.[0-9]+\\.[0-9]+\\.[0-9]+[:/]",
    "23[0-9]\\.[0-9]+\\.[0-9]+\\.[0-9]+[:/]",
    "2[45][0-9]\\.[0-9]+\\.[0-9]+\\.[0-9]+[:/]",
    // IPv6 outside 2000::/3: a leading `::` (unspecified, loopback, IPv4-mapped and -compatible), a first group of
    // one to three digits (below 0x1000: NAT64 `64:ff9b::`, discard `100::`) or of four digits not starting with 2
    // or 3 (unique local, link-local, site-local, multicast).
    "\\[:",
    "\\[[0-9a-f]:",
    "\\[[0-9a-f][0-9a-f]:",
    "\\[[0-9a-f][0-9a-f][0-9a-f]:",
    "\\[[014-9a-f][0-9a-f][0-9a-f][0-9a-f]:",
    // Inside 2000::/3 but embedding or documenting: 6to4, Teredo / IETF protocol space, documentation.
    "\\[2002:",
    "\\[2001::",
    "\\[2001:[01]?[0-9a-f]?[0-9a-f]:",
    "\\[2001:db8:",
    "\\[3fff::",
    "\\[3fff:[0-9a-f]?[0-9a-f]?[0-9a-f]:",
    // Names that only ever mean the local network.
    "[^/:@]*\\.local\\.?[:/]",
    "[^/:@]*\\.home\\.arpa\\.?[:/]",
    "[^/:@]*\\.internal\\.?[:/]",
];

/// The loopback spellings a page of the session's own server may use (the egress guard's three).
const LOOPBACK_SPELLINGS: &[&str] = &["127\\.0\\.0\\.1", "localhost", "\\[::1\\]"];

fn block(filter: &str) -> Value {
    json!({"trigger": {"url-filter": filter}, "action": {"type": "block"}})
}

fn allow(filter: &str) -> Value {
    json!({"trigger": {"url-filter": filter}, "action": {"type": "ignore-previous-rules"}})
}

/// `text` with every character a rule pattern treats specially escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if matches!(
            c,
            '.' | '['
                | ']'
                | '('
                | ')'
                | '*'
                | '+'
                | '?'
                | '^'
                | '$'
                | '\\'
                | '/'
                | '{'
                | '}'
                | '|'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The exceptions for loopback `port` (each spelling with `:port`, and without a port where it is the scheme's
/// default: canonical URLs drop `:80` for http/ws and `:443` for https/wss).
fn loopback_exceptions(port: u16, rules: &mut Vec<Value>) {
    let defaults: &[&str] = match port {
        80 => &["http", "ws"],
        443 => &["https", "wss"],
        _ => &[],
    };
    for host in LOOPBACK_SPELLINGS {
        rules.push(allow(&format!("{EXCEPTION_START}{host}:{port}/")));
        for scheme in defaults {
            rules.push(allow(&format!("^{scheme}://{host}/")));
        }
    }
}

/// The rule list (`WKContentRuleList` JSON) for a mount of `page`.
#[must_use]
pub fn content_rules(page: &RulePage) -> Value {
    let mut rules: Vec<Value> = BLOCKED_HOSTS
        .iter()
        .map(|host| block(&format!("{START}{host}")))
        .collect();
    rules.push(block(USER_INFO));
    match page {
        RulePage::Web => {}
        RulePage::Local { ports } => {
            let mut ports = ports.clone();
            ports.sort_unstable();
            ports.dedup();
            for port in ports.into_iter().filter(|p| *p != 0) {
                loopback_exceptions(port, &mut rules);
            }
        }
        RulePage::Lan { host } => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            if !host.is_empty() {
                let host = escape(&host);
                rules.push(allow(&format!("{EXCEPTION_START}{host}/")));
                rules.push(allow(&format!("{EXCEPTION_START}{host}:[0-9]+/")));
            }
        }
    }
    Value::Array(rules)
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
