//! Wire types of the `importSetup` command: detect other agents' setups, scan them, bring the picked rows, undo a
//! bring.
//! Depends on: vw-import (plan rows). Called by: `command`, `command_result`, `import_setup`.

use serde::{Deserialize, Serialize};
use vw_import::Found;

/// One picked row: its id and the fingerprint the person saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ImportPick {
    /// The row's id.
    pub id: String,
    /// Its fingerprint when shown.
    pub fingerprint: String,
}

/// An agent `detect` found, with the name the window shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ImportDetected {
    /// Which agent.
    pub agent: vw_import::Agent,
    /// Its name.
    pub label: String,
}

/// An `importSetup` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ImportRequest {
    /// Which other agents have a personal setup worth offering, by existence checks only (no file is read): the
    /// first-start hint asks this once.
    Detect {},
    /// What the other agents' setups hold (read only).
    Scan {
        /// A project folder the person picked, for its own files.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        project: Option<String>,
    },
    /// Brings the picked rows after the native confirmation that lists every file written.
    Apply {
        /// The rows to bring.
        picks: Vec<ImportPick>,
        /// The project the scan used.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        project: Option<String>,
    },
    /// The newest bring of this run that can still be undone, so the window shows its Undo again after the Import
    /// card was left.
    Last {},
    /// Puts back what a bring changed (within 24 hours).
    Undo {
        /// The bring's id (from its summary).
        bring: String,
    },
}

/// What a bring did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    /// Its id, for Undo.
    pub bring: String,
    /// Rows brought as they were.
    pub brought: u32,
    /// Rows brought with a change.
    pub changed: u32,
    /// Picked rows that could not be brought now, each with why.
    pub left: Vec<String>,
}

/// An `importSetup` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ImportResult {
    /// The agents found by `detect`, in a fixed order; empty when none.
    Detected {
        /// The agents, each with the name shown for it.
        agents: Vec<ImportDetected>,
    },
    /// The plan.
    Plan {
        /// Every row found.
        rows: Vec<Found>,
    },
    /// A bring finished.
    Brought {
        /// What it did.
        summary: ImportSummary,
    },
    /// The newest bring of this run that can still be undone; `None` when there is none.
    Last {
        /// What it did.
        summary: Option<ImportSummary>,
    },
    /// An undo finished.
    Undone {
        /// The bring that was put back.
        bring: String,
    },
}
