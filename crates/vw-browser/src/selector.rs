//! Typed, read-only element selectors for browser observations. Called by the tool boundary and reader.
//! Matching stays in the DOM reader; indices select from the returned document/frame order, never execute input.
use serde::{Deserialize, Serialize};

/// A selector over document elements and open shadow roots (`XPath` follows document semantics).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selector {
    /// Matching method: css, xpath, alt, title or testId.
    pub kind: SelectorKind,
    /// CSS/XPath expression or exact attribute value.
    pub value: String,
    /// Optional zero-based match from the first 200 matches, main document before child frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}

/// Supported selector languages; none evaluates page JavaScript.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectorKind {
    /// CSS selector, also matched inside open shadow roots.
    Css,
    /// `XPath` returning element nodes.
    Xpath,
    /// Exact alternative text.
    Alt,
    /// Exact title attribute.
    Title,
    /// Exact data-testid attribute.
    TestId,
}

impl Selector {
    /// Check bounded input before entering any page.
    /// # Errors
    /// A reason and correction for an empty, oversized or out-of-range selector.
    pub fn validate(&self) -> Result<(), String> {
        if self.value.is_empty() || self.value.len() > 2000 {
            return Err("The selector is empty or too long; supply 1 to 2000 bytes.".into());
        }
        if self.index.is_some_and(|n| n >= crate::model::MAX_REFS) {
            return Err("The selector index exceeds 199; narrow the selector first.".into());
        }
        Ok(())
    }
}
