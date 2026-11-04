//! Ordered WebKit rule evaluation for the native Windows request barrier; independent of any webview.
//! Compiles once per mode; mandatory regex literals index URL candidates before checking page constraints.
use std::collections::HashMap;

use aho_corasick::AhoCorasick;
use regex::bytes::{Regex, RegexBuilder};
use serde::Deserialize;
use url::Url;

/// One native filtering decision, including document cosmetic selectors.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Decision {
    /// Whether the subrequest must be refused before sending.
    pub blocked: bool,
    /// Selectors to hide in this document.
    pub selectors: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    trigger: Trigger,
    action: Action,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct Trigger {
    url_filter: String,
    #[serde(default)]
    url_filter_is_case_sensitive: bool,
    #[serde(default)]
    resource_type: Vec<String>,
    #[serde(default)]
    load_type: Vec<String>,
    #[serde(default)]
    if_domain: Vec<String>,
    #[serde(default)]
    unless_domain: Vec<String>,
    #[serde(default)]
    if_top_url: Vec<String>,
    #[serde(default)]
    unless_top_url: Vec<String>,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum Action {
    Block,
    IgnorePreviousRules,
    CssDisplayNone { selector: String },
}
struct Compiled {
    rule: Rule,
    url: Regex,
    if_top: Vec<Regex>,
    unless_top: Vec<Regex>,
}

/// Compiled rules shared by requests of a Browser webview. No list parsing occurs on the request path.
pub struct Matcher {
    rules: Vec<Compiled>,
    literals: AhoCorasick,
    indexed: Vec<Vec<usize>>,
    unindexed: Vec<usize>,
    network_unindexed: Vec<usize>,
}

fn regex(pattern: &str, sensitive: bool) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .unicode(false)
        .case_insensitive(!sensitive)
        .build()
        .map_err(|e| format!("invalid content rule expression: {e}"))
}

impl Matcher {
    /// Parses exactly the `WebKit` JSON produced by Balanced/Strict, rejecting unknown actions and conditions.
    /// # Errors
    /// Invalid or unsupported rules; the existing filter remains installed.
    pub fn new(json: &str) -> Result<Self, String> {
        if json.len() > super::MAX_LIST_BYTES {
            return Err("content rules are too large".into());
        }
        let source: Vec<Rule> = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut rules = Vec::with_capacity(source.len());
        let mut tokens = HashMap::<String, usize>::new();
        let mut indexed: Vec<Vec<usize>> = Vec::new();
        let mut unindexed = Vec::new();
        for (id, rule) in source.into_iter().enumerate() {
            let t = &rule.trigger;
            if t.resource_type.iter().any(|s| {
                ![
                    "document",
                    "image",
                    "style-sheet",
                    "script",
                    "font",
                    "raw",
                    "svg-document",
                    "media",
                    "popup",
                ]
                .contains(&s.as_str())
            }) || t
                .load_type
                .iter()
                .any(|s| !["first-party", "third-party"].contains(&s.as_str()))
            {
                return Err("unsupported content rule type".into());
            }
            let url = regex(&t.url_filter, t.url_filter_is_case_sensitive)?;
            let if_top = t
                .if_top_url
                .iter()
                .map(|s| regex(s, false))
                .collect::<Result<_, _>>()?;
            let unless_top = t
                .unless_top_url
                .iter()
                .map(|s| regex(s, false))
                .collect::<Result<_, _>>()?;
            if let Some(token) = mandatory_literal(&t.url_filter) {
                let next = tokens.len();
                let slot = *tokens.entry(token).or_insert(next);
                if slot == indexed.len() {
                    indexed.push(Vec::new());
                }
                indexed[slot].push(id);
            } else {
                unindexed.push(id);
            }
            rules.push(Compiled {
                rule,
                url,
                if_top,
                unless_top,
            });
        }
        let mut patterns = vec![String::new(); tokens.len()];
        for (token, slot) in tokens {
            patterns[slot] = token;
        }
        let literals = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(patterns)
            .map_err(|e| e.to_string())?;
        let network_unindexed = unindexed
            .iter()
            .copied()
            .filter(|id| !matches!(rules[*id].rule.action, Action::CssDisplayNone { .. }))
            .collect();
        Ok(Self {
            rules,
            literals,
            indexed,
            unindexed,
            network_unindexed,
        })
    }

    /// Whether this is Off (no native or cosmetic rules).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Applies rules in source order. Top-level documents are always exempt from network blocking.
    /// `page` is the native top page URL, never a page-supplied Referer. Types use `WebKit` spelling.
    #[must_use]
    pub fn decide(&self, uri: &str, kind: &str, page: &str, top: bool) -> Decision {
        self.evaluate(uri, kind, page, top, true)
    }

    /// Network-only fast path: cosmetic selectors are computed once per document, never per subrequest.
    #[must_use]
    pub fn blocks(&self, uri: &str, kind: &str, page: &str, top: bool) -> bool {
        !top && self.evaluate(uri, kind, page, false, false).blocked
    }

    fn evaluate(&self, uri: &str, kind: &str, page: &str, top: bool, cosmetics: bool) -> Decision {
        let (Ok(url), Ok(page)) = (Url::parse(uri), Url::parse(page)) else {
            return Decision::default();
        };
        if url.scheme() == "data" {
            return Decision::default();
        }
        let first_party = url.origin() == page.origin();
        let host = page.host_str().unwrap_or_default();
        let mut candidates = if cosmetics && kind == "document" {
            self.unindexed.clone()
        } else {
            self.network_unindexed.clone()
        };
        for found in self.literals.find_overlapping_iter(url.as_str()) {
            candidates.extend(&self.indexed[found.pattern().as_usize()]);
        }
        candidates.sort_unstable();
        candidates.dedup();
        let mut answer = Decision::default();
        for id in candidates {
            let c = &self.rules[id];
            let t = &c.rule.trigger;
            if (!t.resource_type.is_empty() && !t.resource_type.iter().any(|s| s == kind))
                || (!t.load_type.is_empty()
                    && !t
                        .load_type
                        .iter()
                        .any(|s| (s == "first-party") == first_party))
                || (!t.if_domain.is_empty() && !t.if_domain.iter().any(|d| domain(d, host)))
                || t.unless_domain.iter().any(|d| domain(d, host))
                || (!c.if_top.is_empty()
                    && !c
                        .if_top
                        .iter()
                        .any(|r| r.is_match(page.as_str().as_bytes())))
                || c.unless_top
                    .iter()
                    .any(|r| r.is_match(page.as_str().as_bytes()))
                || !c.url.is_match(url.as_str().as_bytes())
            {
                continue;
            }
            match &c.rule.action {
                Action::Block => answer.blocked = !top,
                Action::IgnorePreviousRules => answer = Decision::default(),
                Action::CssDisplayNone { selector } if cosmetics && kind == "document" => {
                    answer.selectors.push(selector.clone());
                }
                Action::CssDisplayNone { .. } => {}
            }
        }
        answer
    }
}

fn domain(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix('*') {
        Some(base) => host == base || host.strip_suffix(base).is_some_and(|s| s.ends_with('.')),
        None => pattern == host,
    }
}

/// A literal that MUST appear in every match. Optional repetitions/alternations cannot supply one.
/// Parse case-sensitively to retain literal runs; the index itself folds ASCII case (safe over-selection).
fn mandatory_literal(pattern: &str) -> Option<String> {
    use regex_syntax::hir::{Hir, HirKind};
    fn longest(hir: &Hir) -> Option<&[u8]> {
        match hir.kind() {
            HirKind::Literal(l) => Some(&l.0),
            HirKind::Capture(c) => longest(&c.sub),
            HirKind::Repetition(r) if r.min > 0 => longest(&r.sub),
            HirKind::Concat(parts) => parts.iter().filter_map(longest).max_by_key(|s| s.len()),
            _ => None,
        }
    }
    let hir = regex_syntax::ParserBuilder::new()
        .unicode(false)
        .utf8(false)
        .build()
        .parse(pattern)
        .ok()?;
    let literal = longest(&hir)?;
    (literal.len() >= 3 && literal.is_ascii())
        .then(|| String::from_utf8_lossy(literal).to_ascii_lowercase())
}

#[cfg(test)]
#[path = "matcher_tests.rs"]
mod tests;
