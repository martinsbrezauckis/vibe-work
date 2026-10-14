//! The `safety` command's requests and results: Settings > Safety lists the safety
//! reminders, switches a built-in one off or on, and adds or removes the user's own ("the command contains ...").
//! Logic in `safety.rs`; the rules in vw-agent-tools `safety`.
//!
//! Depends on: serde. Called by: `command` (the `safety` variant), `safety`, the UI through the generated TypeScript.
//! Invariant: wire form `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.

use serde::{Deserialize, Serialize};

/// One `safety` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SafetyRequest {
    /// The rules and their state.
    Read {},
    /// Switches a built-in rule on or off.
    SetRule {
        /// The rule's id.
        id: String,
        /// On (the agent is stopped) or off.
        on: bool,
    },
    /// Adds a rule of the user's own: a command that contains `text` (case-insensitive) is stopped.
    AddCustom {
        /// The text (1 to 200 characters).
        text: String,
    },
    /// Removes a rule of the user's own.
    RemoveCustom {
        /// Its id.
        id: String,
    },
}

/// A built-in rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SafetyRuleView {
    /// Stable id.
    pub id: String,
    /// What it stops.
    pub label: String,
    /// On.
    pub on: bool,
}

/// A rule of the user's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CustomRuleView {
    /// Stable id.
    pub id: String,
    /// The text a command must contain.
    pub text: String,
}

/// One `safety` answer: the whole state after the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SafetyResult {
    /// False when the agent tools are not running (nothing can be checked; the settings are kept anyway).
    pub active: bool,
    /// The built-in rules, in order.
    pub rules: Vec<SafetyRuleView>,
    /// The user's own rules.
    pub custom: Vec<CustomRuleView>,
}
