//! Manual pairing fallbacks keep a bare URL unless a pending automatic mint expired.
//! Uses the offline enrollment rig and paused clock; called by the enrollment test suite.
use super::*;
use std::time::Duration;

#[tokio::test(start_paused = true)]
async fn missing_attempt_and_exhausted_mint_keep_the_bare_pair_page() {
    for missing in [true, false] {
        for seconds in [0, 601] {
            let rig = Rig::new(
                &format!("chrome-enroll-no-mint-{missing}-{seconds}"),
                ticket_answer,
            )
            .await;
            rig.connect(Origin::Ui).await;
            {
                let mut enroll = rig.core.shared.chrome_control.owner.enroll.lock().unwrap();
                if missing {
                    enroll.cancel();
                } else {
                    enroll.attempt.as_mut().unwrap().mint_on_waiting = false;
                }
            }
            tokio::time::advance(Duration::from_secs(seconds)).await;
            let mut expected = rig.opened();
            expected.push(plain());
            rig.waiting(true);
            rig.settle().await;
            assert_eq!(rig.opened(), expected);
            assert_eq!(rig.methods(), [enroll::REVOKE]);
            assert!(!has_attempt(&rig));
            rig.waiting(true);
            rig.settle().await;
            assert_eq!(rig.opened(), expected);
            rig.core.shutdown().await;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn unsupported_enrollment_and_owner_mismatch_keep_the_bare_pair_page() {
    for mismatch in [true, false] {
        for seconds in [0, 601] {
            let rig = Rig::new(
                &format!("chrome-enroll-manual-{mismatch}-{seconds}"),
                ticket_answer,
            )
            .await;
            rig.connect(Origin::Ui).await;
            tokio::time::advance(Duration::from_secs(seconds)).await;
            let mut expected = rig.opened();
            expected.push(plain());
            let params = json!({
                "profileId":PROFILE,
                "state":if mismatch { "mismatch" } else { "waiting" },
                "fingerprint":OURS,
                "enroll":u8::from(mismatch)
            });
            for _ in 0..2 {
                owner::observe(
                    &Arc::downgrade(&rig.core.shared),
                    &json!({"method":"notifications/owner/pairing","params":params}),
                )
                .unwrap();
                rig.settle().await;
                assert_eq!(rig.opened(), expected);
                assert_eq!(rig.methods(), [enroll::REVOKE]);
                assert!(!has_attempt(&rig));
            }
            rig.core.shutdown().await;
        }
    }
}

#[tokio::test]
async fn mint_owner_fingerprint_mismatch_keeps_the_bare_pair_page() {
    let rig = Rig::new("chrome-enroll-mint-mismatch", |name, _| {
        if name == enroll::MINT {
            json!({"ticket":TICKET,"profileId":PROFILE,"fingerprint":"b".repeat(64)})
        } else {
            json!({})
        }
    })
    .await;
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    assert_eq!(rig.opened(), [plain()]);
    assert_eq!(rig.methods(), [enroll::REVOKE, enroll::MINT]);
    assert!(!has_attempt(&rig));
    rig.core.shutdown().await;
}
