//! Browser content rules (macOS `WKContentRuleList`): URL credentials are refused for every page; a page from the web
//! does not reach this computer through an address literal unless the person allowed its site (Local Network Access).
//! `WebKit` on macOS sends `127.0.0.1`, `[::1]`, `localhost`, this computer's own interface addresses and link-local
//! literals (`169.254.0.0/16`, `fe80::/10`) around the guard's proxy, so the guard's question (and its refusal of a
//! blocked site) can never see those requests; the rules hold them instead. This computer's own interface addresses
//! are held for web pages even when a site was allowed: an allowed frame cannot be attributed safely when another
//! undecided site shares the store. Loopback and link-local retain the allowed-site frame exception. Other literals
//! and every host name go through the proxy, where the person is asked. Vibe Work's own ports ([`set_internal_ports`]) are held for every page,
//! whatever the person answered, on every spelling of this computer; every guard's listeners ([`crate::listeners`])
//! are among them.
//! Called by native webviews; generation tracking prevents stale asynchronous rule installs. The inputs of the lists
//! (allowed sites, Vibe Work's own ports) are process-wide; [`revision`] changes whenever one of them changes, so the
//! owner of the mounted webviews can install their lists again.
//! The App preview's requests to this computer reach only the chats' server ports on loopback ([`RulePage::App`]).
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError, RwLock};

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
    /// The App preview: its requests to this computer reach only the chats' servers on these loopback ports; every
    /// other port, this computer's other addresses and link-local addresses are held (Vibe Work's own ports never
    /// among them).
    App {
        /// The chats' server ports, any order.
        ports: Vec<u16>,
    },
    /// The local-network page the user opened on `host` (lowercase; IPv6 in brackets).
    Lan {
        /// The page's host as it appears in its URL.
        host: String,
    },
}

/// The sites the person allowed for Local Network Access (origin text), for the web pages' rules
/// ([`set_allowed_sites`]).
static ALLOWED_SITES: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Vibe Work's own ports ([`set_internal_ports`]), sorted.
static INTERNAL_PORTS: RwLock<Vec<u16>> = RwLock::new(Vec::new());

/// Bumped by every change of an input of the lists ([`revision`]).
static REVISION: AtomicU64 = AtomicU64::new(0);

/// What a list is generated from besides its [`RulePage`] ([`content_rules_for`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuleInputs {
    /// The sites the person allowed (origin text such as `https://example.com`).
    pub allowed_sites: Vec<String>,
    /// Vibe Work's own ports: held for every page.
    pub internal_ports: Vec<u16>,
    /// This computer's own interface addresses (`own`): spellings of this computer like loopback.
    pub own_addresses: Vec<std::net::IpAddr>,
}

/// Address literals of this computer, one `url-filter` each (content-rule regexes have no alternation). The URL is
/// normalized by the engine first (`127.1`, `0x7f.1`, `%31%32%37.0.0.1` and `LOCALHOST` arrive as `127.0.0.1` and
/// `localhost`); a trailing dot is its own spelling.
const LOOPBACK_FILTERS: &[&str] = &[
    r"^[a-z]+://127\.[0-9]+\.[0-9]+\.[0-9]+\.?[:/]",
    r"^[a-z]+://0\.0\.0\.0\.?[:/]",
    r"^[a-z]+://\[::1\][:/]",
    r"^[a-z]+://\[::\][:/]",
    r"^[a-z]+://\[::ffff:7f[0-9a-f:]*\][:/]",
    r"^[a-z]+://\[::7f[0-9a-f:]*\][:/]",
    r"^[a-z]+://localhost\.?[:/]",
    r"^[a-z]+://[^/:]*\.localhost\.?[:/]",
];

/// Link-local literals (`169.254.0.0/16` with the IPv6 spellings that embed it: IPv4-mapped, IPv4-compatible, NAT64;
/// `fe80::/10`), the host part only: a page from the web is held from them like from this computer (they never reach
/// the proxy either).
const LINK_LOCAL_HOSTS: &[&str] = &[
    r"^[a-z]+://169\.254\.[0-9]+\.[0-9]+\.?",
    r"^[a-z]+://\[fe[89ab][0-9a-f]:[0-9a-f:]*\]",
    r"^[a-z]+://\[::ffff:a9fe:[0-9a-f]*\]",
    r"^[a-z]+://\[::a9fe:[0-9a-f]*\]",
    r"^[a-z]+://\[64:ff9b::a9fe:[0-9a-f]*\]",
];

/// Every request type a page makes by itself, in `WebKit`'s content-rule names: `raw` covers fetch, XHR,
/// WebSocket, beacons and pings; `csp-report` is a Content-Security-Policy violation report, which `raw` does not
/// cover (`WebKit` `ResourceLoadInfo.cpp`); `other` is anything else; `popup` a window the page opens. A frame is a
/// `document` in a child frame. The page itself (the top frame) is never held: opening a local page remounts it as a
/// local page. A test pins this exact set, so a type cannot be left out unnoticed.
const SUBRESOURCES: &[&str] = &[
    "image",
    "style-sheet",
    "script",
    "font",
    "raw",
    "svg-document",
    "media",
    "popup",
    "ping",
    "fetch",
    "websocket",
    "other",
    "csp-report",
];

/// Sets the sites the person allowed (origin text such as `https://example.com`); web pages mounted from now on
/// get rules that let those sites reach this computer.
pub fn set_allowed_sites(sites: Vec<String>) {
    let mut held = ALLOWED_SITES
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    if *held != sites {
        *held = sites;
        inputs_changed();
    }
}

/// Moves [`revision`] (an input changed: here or in `own`).
pub(crate) fn inputs_changed() {
    REVISION.fetch_add(1, Ordering::SeqCst);
}

/// Sets Vibe Work's own ports (every listener of the app a page must never reach); every page mounted from now on
/// gets rules that hold them. Returns true when the list changed ([`revision`] moved).
pub fn set_internal_ports(mut ports: Vec<u16>) -> bool {
    ports.sort_unstable();
    ports.dedup();
    let mut held = INTERNAL_PORTS
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    if *held == ports {
        return false;
    }
    *held = ports;
    inputs_changed();
    true
}

/// A number that changes whenever an input of the lists changes: compare it with the value read when the mounted
/// webviews' lists were last installed.
#[must_use]
pub fn revision() -> u64 {
    REVISION.load(Ordering::SeqCst)
}

/// The process-wide inputs now.
#[must_use]
pub fn current_inputs() -> RuleInputs {
    RuleInputs {
        allowed_sites: ALLOWED_SITES
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        internal_ports: {
            // The published ports plus every guard's listeners (a guard that never publishes is held too).
            let mut ports = INTERNAL_PORTS
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            ports.extend(crate::listeners::ports());
            ports.sort_unstable();
            ports.dedup();
            ports
        },
        own_addresses: crate::own::own_addresses(),
    }
}

/// A top-URL pattern for one site: the origin followed by the start of a path (so `https://a.example` never
/// matches `https://a.example.evil`).
fn top_url(site: &str) -> String {
    let mut pattern = String::from("^");
    for c in site.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | '-') {
            pattern.push(c);
        } else {
            pattern.push('\\');
            pattern.push(c);
        }
    }
    pattern.push('/');
    pattern
}

/// The rules of a page from the web whose allowed sites are `allowed` (see the module docs).
#[must_use]
pub fn web_rules(allowed: &[String]) -> String {
    let inputs = RuleInputs {
        allowed_sites: allowed.to_vec(),
        ..RuleInputs::default()
    };
    content_rules_for(&RulePage::Web, &inputs).to_string()
}

/// The host part (everything before the port or path) of every spelling of this computer: each
/// [`LOOPBACK_FILTERS`] entry, then each of this computer's own interface addresses as the engine writes it in a URL
/// (IPv6 in brackets, RFC 5952 form; a trailing dot is its own spelling of an IPv4 address).
fn this_computer_hosts(own: &[std::net::IpAddr]) -> Vec<String> {
    let mut hosts: Vec<String> = LOOPBACK_FILTERS
        .iter()
        .map(|f| f.strip_suffix("[:/]").unwrap_or(f).to_owned())
        .collect();
    for ip in own {
        let text = ip.to_string().replace('.', r"\.");
        match ip {
            std::net::IpAddr::V4(v4) => {
                hosts.push(format!(r"^[a-z]+://{text}\.?"));
                // The IPv6 spellings that embed it, as the engine serializes them (hex groups).
                let [a, b, c, d] = v4.octets();
                let (high, low) = (u16::from_be_bytes([a, b]), u16::from_be_bytes([c, d]));
                hosts.push(format!(r"^[a-z]+://\[::ffff:{high:x}:{low:x}\]"));
                hosts.push(format!(r"^[a-z]+://\[::{high:x}:{low:x}\]"));
            }
            std::net::IpAddr::V6(_) => hosts.push(format!(r"^[a-z]+://\[{text}\]")),
        }
    }
    hosts
}

/// Block rules for Vibe Work's own ports on every spelling of this computer, for every resource of every frame
/// (the top frame included): no site is excepted.
fn internal_rules(rules: &mut Vec<Value>, hosts: &[String], ports: &[u16]) {
    for port in ports {
        for host in hosts {
            rules.push(json!({
                "trigger": {"url-filter": format!("{host}:{port}[/?#]")},
                "action": {"type": "block"},
            }));
        }
    }
}

/// The held requests of a page: its subresources and child-frame documents to `host`, plus `extra` trigger fields.
fn held_triggers(host: &str, extra: &Value) -> [Value; 2] {
    let filter = format!("{host}[:/]");
    let mut triggers = [
        json!({"url-filter": filter, "resource-type": SUBRESOURCES}),
        json!({"url-filter": filter, "resource-type": ["document"], "load-context": ["child-frame"]}),
    ];
    if let (Some(fields), true) = (extra.as_object(), true) {
        for trigger in &mut triggers {
            for (key, value) in fields {
                trigger[key] = value.clone();
            }
        }
    }
    triggers
}

/// A page from the web: `hosts` held, except for a request made by a frame (the page itself or a frame inside it)
/// whose own URL is on an allowed site. A frame of another site inside an allowed page stays held: its own frame URL
/// decides, never the top page's.
fn web_rule_list(
    rules: &mut Vec<Value>,
    hosts: &[String],
    own_hosts: &[String],
    allowed: &[String],
) {
    let unless: Vec<String> = allowed
        .iter()
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .map(|s| top_url(s))
        .collect();
    let extra = if unless.is_empty() {
        Value::Null
    } else {
        json!({"unless-frame-url": unless})
    };
    for host in hosts {
        for trigger in held_triggers(host, &extra) {
            rules.push(json!({"trigger": trigger, "action": {"type": "block"}}));
        }
    }
    for host in own_hosts {
        for trigger in held_triggers(host, &Value::Null) {
            rules.push(json!({"trigger": trigger, "action": {"type": "block"}}));
        }
    }
}

/// The App preview: `hosts` held, then lifted again for the chats' server `ports` on the loopback spellings only
/// (`ignore-previous-rules`; later rules apply again).
fn app_rule_list(rules: &mut Vec<Value>, hosts: &[String], ports: &[u16]) {
    for host in hosts {
        for trigger in held_triggers(host, &Value::Null) {
            rules.push(json!({"trigger": trigger, "action": {"type": "block"}}));
        }
    }
    let loopback: Vec<&str> = LOOPBACK_FILTERS
        .iter()
        .map(|f| f.strip_suffix("[:/]").unwrap_or(f))
        .collect();
    for port in ports {
        let mut filters: Vec<String> = loopback
            .iter()
            .map(|host| format!("{host}:{port}[/?#]"))
            .collect();
        // The engine writes a scheme's default port nowhere in the URL.
        let defaults: &[&str] = match port {
            80 => &["http", "ws"],
            443 => &["https", "wss"],
            _ => &[],
        };
        for scheme in defaults {
            filters.extend(
                loopback
                    .iter()
                    .map(|host| format!("{}[/?#]", host.replacen("[a-z]+", scheme, 1))),
            );
        }
        for filter in filters {
            rules.push(json!({
                "trigger": {"url-filter": filter},
                "action": {"type": "ignore-previous-rules"},
            }));
        }
    }
}

fn credential_rule() -> Value {
    json!({"trigger":{"url-filter":"^[a-z]+://[^/]*@"},"action":{"type":"block"}})
}

/// The content rules of `page` for the process-wide inputs now ([`content_rules_for`]).
#[must_use]
pub fn content_rules(page: &RulePage) -> Value {
    content_rules_for(page, &current_inputs())
}

/// The content rules of `page` for `inputs`: the rules of a page from the web or of the App preview for every
/// spelling of this computer (loopback and its own interface addresses) and link-local literals, then credentials
/// and Vibe Work's own ports for every page. Those come last: no earlier `ignore-previous-rules` lifts them.
#[must_use]
pub fn content_rules_for(page: &RulePage, inputs: &RuleInputs) -> Value {
    let mut rules = Vec::new();
    let hosts = this_computer_hosts(&inputs.own_addresses);
    let mut held = hosts[..LOOPBACK_FILTERS.len()].to_vec();
    held.extend(LINK_LOCAL_HOSTS.iter().map(|h| (*h).to_owned()));
    match page {
        RulePage::Web => web_rule_list(
            &mut rules,
            &held,
            &hosts[LOOPBACK_FILTERS.len()..],
            &inputs.allowed_sites,
        ),
        RulePage::App { ports } => {
            held.extend(hosts[LOOPBACK_FILTERS.len()..].iter().cloned());
            app_rule_list(&mut rules, &held, ports);
        }
        RulePage::Local { .. } | RulePage::Lan { .. } => {}
    }
    rules.push(credential_rule());
    internal_rules(&mut rules, &hosts, &inputs.internal_ports);
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
