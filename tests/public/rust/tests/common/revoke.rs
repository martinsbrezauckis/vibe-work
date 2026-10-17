//! Observable revoke barriers for grant tests. Reads metadata through KeyService and the same
//! SystemTime clock used by dialog timestamps; callers start a new ask only after its revoke.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vw_keys::KeyService;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap()
}

pub async fn clock_after(at: i64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while now_ms() <= at {
        assert!(
            Instant::now() < deadline,
            "clock never passed revoke timestamp {at}"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

pub async fn after_revoke(service: &KeyService, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = if vw_vault::command_grant::is_grant_id(id) {
            service
                .command_grant(id.to_owned())
                .await
                .unwrap()
                .map(|g| (!g.active(), g.revoked_ms))
        } else if vw_vault::fill_grant::is_grant_id(id) {
            service
                .fill_grant(id.to_owned())
                .await
                .unwrap()
                .map(|g| (!g.active(), g.revoked_ms))
        } else {
            service
                .mcp_binding(id.to_owned())
                .await
                .unwrap()
                .map(|g| (!g.always, g.revoked_ms))
        };
        if let Some((true, revoked)) = state.filter(|(_, revoked)| *revoked > 0) {
            clock_after(revoked).await;
            return;
        }
        assert!(
            Instant::now() < deadline,
            "revoke of {id} never became visible"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

pub async fn dialog_after(count: impl Fn() -> usize, previous: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while count() == previous {
        assert!(Instant::now() < deadline, "dialog never became visible");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}
