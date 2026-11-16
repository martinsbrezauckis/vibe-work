//! PREDICT: before the status line existed, these do not compile (`status_line`, `PrReason` missing). They pin
//! Vibe 3.1.2's order (`forge_sync.derive_pr_status`) and the required/optional split.

use super::{PrReason, mark_required, status_line};
use crate::pr::{CheckItem, Checks, Mergeable, PrCard, PrState, Review};

fn card() -> PrCard {
    PrCard {
        number: 7,
        url: "https://github.com/o/r/pull/7".to_owned(),
        title: "t".to_owned(),
        state: PrState::Open,
        draft: false,
        review: Review::None,
        mergeable: Mergeable::Yes,
        checks: Checks::default(),
        head: "feature".to_owned(),
        base: "main".to_owned(),
        merge_blocked: false,
        status: super::PrStatusLine::default(),
    }
}

fn check(name: &str, outcome: &str) -> CheckItem {
    CheckItem {
        name: name.to_owned(),
        outcome: outcome.to_owned(),
        url: None,
        required: None,
    }
}

#[test]
fn status_reason_matches_vibe_order() {
    let mut all = card();
    all.review = Review::ChangesRequested;
    all.checks.items = vec![check("ci", "failure"), check("lint", "pending")];
    all.mergeable = Mergeable::Conflicting;
    all.merge_blocked = true;
    let mut cases = vec![(all.clone(), Some(PrReason::ChangesRequested))];
    all.review = Review::Required;
    cases.push((all.clone(), Some(PrReason::ChecksFailing)));
    all.checks.items.remove(0);
    cases.push((all.clone(), Some(PrReason::Conflicts)));
    all.mergeable = Mergeable::Unknown;
    cases.push((all.clone(), Some(PrReason::ChecksPending)));
    all.checks.items.clear();
    cases.push((all.clone(), Some(PrReason::ReviewRequired)));
    all.review = Review::Approved;
    cases.push((all.clone(), Some(PrReason::MergeBlocked)));
    all.merge_blocked = false;
    cases.push((all.clone(), None));
    all.mergeable = Mergeable::Yes;
    cases.push((all.clone(), Some(PrReason::Ready)));
    for (i, (card, want)) in cases.into_iter().enumerate() {
        assert_eq!(status_line(&card).reason, want, "case {i}");
    }
}

#[test]
fn closed_merged_and_draft_have_no_status_line() {
    for change in [
        |c: &mut PrCard| c.state = PrState::Closed,
        |c: &mut PrCard| c.state = PrState::Merged,
        |c: &mut PrCard| c.draft = true,
    ] {
        let mut c = card();
        c.checks.items = vec![check("ci", "failure")];
        change(&mut c);
        assert_eq!(status_line(&c).reason, None);
    }
}

#[test]
fn optional_failing_required_passing_is_ready_with_a_note() {
    let mut c = card();
    c.checks.items = vec![check("docs", "failure")];
    mark_required(&mut c, &["build".to_owned()]);
    assert!(c.checks.required_known);
    assert_eq!(c.status.reason, Some(PrReason::Ready));
    assert!(c.status.optional_failing);
    // Unknown split (public API): every check counts as required.
    let mut unknown = card();
    unknown.checks.items = vec![check("docs", "failure")];
    assert_eq!(status_line(&unknown).reason, Some(PrReason::ChecksFailing));
    assert!(!unknown.checks.required_known);
}
