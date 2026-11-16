//! The one status line of an open pull request and its cause, in Vibe's own order (Vibe 3.1.2
//! `forge_sync.derive_pr_status`): changes requested, required checks failing, conflicts with the base, required
//! checks still running, review required, merge rules; otherwise ready to merge. A closed, merged or draft pull
//! request has no status line (its state is its label). Only required checks count; failing optional checks are
//! a separate note. When the required checks are not known (the public API cannot read branch protection),
//! every check counts as required and the line says the split is unknown.
//!
//! Depends on: `pr` (the card). Called by: `pr` (when a card is read), vw-core-api (after gh names the required
//! checks). The cause names are Vibe's own (`pr_status_reason`), so a later switch to Vibe's data is mechanical.

use serde::{Deserialize, Serialize};

use crate::pr::{Mergeable, PrCard, PrState, Review};

/// Why an open pull request cannot be merged yet, or that it can.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PrReason {
    /// A reviewer asked for changes.
    ChangesRequested,
    /// A required check failed.
    ChecksFailing,
    /// It conflicts with its base branch.
    Conflicts,
    /// A required check is still running.
    ChecksPending,
    /// A review is required and none was given.
    ReviewRequired,
    /// The forge's merge rules hold it (branch protection).
    MergeBlocked,
    /// Nothing holds it.
    Ready,
}

/// The card's status line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PrStatusLine {
    /// The cause; `None` for a closed, merged or draft pull request, or an open one whose state is not known yet.
    pub reason: Option<PrReason>,
    /// An optional (not required) check failed.
    pub optional_failing: bool,
}

fn failed(outcome: &str) -> bool {
    outcome != "pending"
}

/// The status line of `card`, from its review, checks and mergeability.
#[must_use]
pub fn status_line(card: &PrCard) -> PrStatusLine {
    let required = |item: &crate::pr::CheckItem| item.required != Some(false);
    let items = &card.checks.items;
    let optional_failing = items.iter().any(|i| !required(i) && failed(&i.outcome));
    if card.state != PrState::Open || card.draft {
        return PrStatusLine {
            reason: None,
            optional_failing,
        };
    }
    let checks_failing = items.iter().any(|i| required(i) && failed(&i.outcome));
    let checks_pending = items.iter().any(|i| required(i) && !failed(&i.outcome));
    let rules = [
        (
            card.review == Review::ChangesRequested,
            PrReason::ChangesRequested,
        ),
        (checks_failing, PrReason::ChecksFailing),
        (
            card.mergeable == Mergeable::Conflicting,
            PrReason::Conflicts,
        ),
        (checks_pending, PrReason::ChecksPending),
        (card.review == Review::Required, PrReason::ReviewRequired),
        (card.merge_blocked, PrReason::MergeBlocked),
    ];
    let reason = rules
        .into_iter()
        .find_map(|(holds, reason)| holds.then_some(reason))
        .or((card.mergeable == Mergeable::Yes).then_some(PrReason::Ready));
    PrStatusLine {
        reason,
        optional_failing,
    }
}

/// Marks which of the card's checks are required (`names`, as gh lists them) and recomputes its status line.
pub fn mark_required(card: &mut PrCard, names: &[String]) {
    for item in &mut card.checks.items {
        item.required = Some(names.contains(&item.name));
    }
    card.checks.required_known = true;
    card.status = status_line(card);
}

#[cfg(test)]
#[path = "pr_status_tests.rs"]
mod tests;
