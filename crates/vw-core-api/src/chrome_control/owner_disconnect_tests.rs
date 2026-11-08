//! Owner shutdown and queued-request deadlines; uses in-memory pipes without launching a helper.
use super::*;

#[tokio::test]
async fn explicit_stop_forgets_pairing_and_fingerprint() {
    let owner = Owner::default();
    *owner.pairing.lock().unwrap() = (PairingState::Paired, Some("0123456789abcdef".to_owned()));
    owner.stop().await;
    assert_eq!(
        *owner.pairing.lock().unwrap(),
        (PairingState::Disconnected, None)
    );
}

#[tokio::test(start_paused = true)]
async fn requests_finish_when_startup_never_reaches_the_queue() {
    for full in [false, true] {
        for rpc in [false, true] {
            let owner = Owner::default();
            let (send, requests) = mpsc::channel(1);
            let (stop, mut stopped) = watch::channel(false);
            let task = tokio::spawn(async move {
                // A supervisor still starting its helper keeps the queue open without receiving calls.
                let _requests = requests;
                let _ = stopped.changed().await;
            });
            *owner.running.lock().unwrap() = Some(Running { send, stop, task });
            if full {
                owner.control(json!({"action":"stop","sessionId":"previous"}));
            }
            let started = tokio::time::Instant::now();
            let result = tokio::time::timeout(Duration::from_secs(36), async {
                if rpc {
                    assert!(
                        owner
                            .rpc("owner.browserAvailability", json!({}))
                            .await
                            .is_none()
                    );
                } else {
                    let answer = owner.call("chrome_status", json!({})).await;
                    assert!(matches!(answer, Answer::Own(_)));
                    assert_eq!(
                        answer.value()["structuredContent"]["error"]["code"],
                        "TRANSPORT_LOST"
                    );
                }
            })
            .await;
            owner.stop().await;
            assert!(
                result.is_ok(),
                "queue full={full}, rpc={rpc}: request must have its own deadline"
            );
            assert_eq!(started.elapsed(), Duration::from_secs(35));
        }
    }
}
