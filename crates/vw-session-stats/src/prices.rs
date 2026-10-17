//! Caller-owned prices and arithmetic; depends on public usage types, used by scanner and core clients.
//! No provider prices are bundled, and a latest config snapshot never prices historical usage.

use crate::{Error, TokenUsage};
use std::collections::BTreeMap;

/// Rates per million tokens in a single caller-chosen currency.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrices {
    input: f64,
    output: f64,
    cached_input: f64,
}

impl ModelPrices {
    /// Construct rates. `cached_input` is explicit, including when equal to the input rate.
    ///
    /// # Errors
    /// Rejects negative or non-finite rates.
    pub fn new(input: f64, output: f64, cached_input: f64) -> Result<Self, Error> {
        if [input, output, cached_input]
            .iter()
            .any(|price| !price.is_finite() || *price < 0.0)
        {
            return Err(Error::InvalidPrice);
        }
        Ok(Self {
            input,
            output,
            cached_input,
        })
    }
}

/// Caller-supplied rates keyed by model ID, with an optional explicit rate for every model.
/// A uniform rate is a caller assertion, not a fallback inferred from a one-entry table.
#[derive(Debug, Clone, Default)]
pub struct PriceTable {
    models: BTreeMap<String, ModelPrices>,
    uniform: Option<ModelPrices>,
}

impl PriceTable {
    /// Set prices for a model ID supplied alongside recorded usage.
    pub fn insert(&mut self, model: impl Into<String>, prices: ModelPrices) {
        self.models.insert(model.into(), prices);
    }

    /// Use these caller-supplied rates for all models, even when the model is unknown.
    /// This makes aggregate stored-session usage priceable without inventing model history.
    #[must_use]
    pub fn uniform(prices: ModelPrices) -> Self {
        Self {
            models: BTreeMap::new(),
            uniform: Some(prices),
        }
    }

    /// Compute cost when usage and applicable rates are known.
    ///
    /// Input tokens include cached tokens, so the cached subset is charged once. Missing cached
    /// usage only permits a cost when both input rates are equal. Invalid usage/overflow yields
    /// unknown. Vibe's stored sessions have no historical model attribution: scanner calls use
    /// `model = None`, which requires an explicit uniform rate. This helper also serves callers
    /// with independently recorded model-specific usage.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // Money uses floating-point rates; token counters remain exact u64.
    #[allow(clippy::float_cmp)] // Only exactly equal rates make missing cached usage irrelevant.
    pub fn cost(&self, model: Option<&str>, usage: TokenUsage) -> Option<f64> {
        let prices = self.uniform.as_ref().or_else(|| self.models.get(model?))?;
        usage.validate().ok()?;
        let input = usage.input?;
        let output = usage.output?;
        let amount = if prices.input == prices.cached_input {
            input as f64 * prices.input + output as f64 * prices.output
        } else {
            let cached = usage.cached?;
            (input - cached) as f64 * prices.input
                + cached as f64 * prices.cached_input
                + output as f64 * prices.output
        } / 1_000_000.0;
        amount.is_finite().then_some(amount)
    }
}
