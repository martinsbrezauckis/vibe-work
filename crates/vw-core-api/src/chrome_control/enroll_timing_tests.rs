//! Enrollment timing through the offline core and recording launcher; no browser opens.
//! Uses the parent rig to keep delayed Enable inside one bounded, single-use Connect attempt.
use super::*;
use std::time::Duration;

#[tokio::test(start_paused = true)]
async fn delayed_enable_still_auto_pairs_once_within_ten_minutes() {
    for seconds in [180, 599] {
        let rig = Rig::new(&format!("chrome-enroll-delayed-{seconds}"), ticket_answer).await;
        rig.connect(Origin::Ui).await;
        tokio::time::advance(Duration::from_secs(seconds)).await;
        rig.waiting(true);
        rig.settle().await;
        assert_eq!(rig.methods(), [enroll::REVOKE, enroll::MINT]);
        assert_eq!(rig.opened().last(), Some(&enroll_url()));
        let opened = rig.opened();
        rig.waiting(true);
        rig.settle().await;
        assert_eq!(rig.methods(), [enroll::REVOKE, enroll::MINT]);
        assert_eq!(
            rig.opened(),
            opened,
            "a repeated waiting report opened again"
        );
        rig.core.shutdown().await;
    }
}

#[tokio::test(start_paused = true)]
async fn delayed_enable_at_or_after_ten_minutes_opens_expired_pair_page_once() {
    for seconds in [600, 601] {
        let rig = Rig::new(&format!("chrome-enroll-expired-{seconds}"), ticket_answer).await;
        rig.connect(Origin::Ui).await;
        let mut expected = rig.opened();
        expected.push(expired_url());
        tokio::time::advance(Duration::from_secs(seconds)).await;
        rig.waiting(true);
        rig.settle().await;
        assert_eq!(rig.methods(), [enroll::REVOKE]);
        assert_eq!(rig.opened(), expected);
        assert!(!has_attempt(&rig));
        let opened = rig.opened();
        rig.waiting(true);
        rig.settle().await;
        assert_eq!(rig.methods(), [enroll::REVOKE]);
        assert_eq!(rig.opened(), opened);
        rig.core.shutdown().await;
    }
}
