//! Safari removal revokes its signed identity before unregistering the extension. Uses the private owner pipe;
//! called under the browser lifecycle lock, and never reports removal without the helper's acknowledgement.
use crate::{CoreError, core::Shared};
use serde_json::json;
use std::sync::{Arc, atomic::Ordering};

pub(super) async fn revoke(shared: &Arc<Shared>) -> Result<(), CoreError> {
    let chrome = &shared.chrome_control;
    if !chrome.owner.is_running() {
        if chrome.owner.credential_missing.load(Ordering::SeqCst) {
            // A missing owner key cannot admit the old signed pairing.
            return Ok(());
        }
        super::owner::start_for_removal(shared)
            .await
            .map_err(|reason| super::commands::invalid(&reason))?;
    }
    let answer = chrome
        .owner
        .rpc("owner.browserUnpair", json!({"browser":"safari"}))
        .await;
    if !answer.is_some_and(|answer| answer["unpaired"] == true) {
        return Err(super::commands::invalid(
            "Safari's pairing could not be removed. Reconnect the helper and try Remove again.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn safari_removal_requires_the_exact_browser_acknowledgement() {
        for acknowledged in [false, true] {
            let core = crate::offline_core::start_with("safari-revoke", |config, dir| {
                config.chrome_control.package = Some(dir.join("package"));
            })
            .await;
            let calls = core
                .shared
                .chrome_control
                .owner
                .answering(move |_, _| json!({"unpaired":acknowledged}));
            assert_eq!(revoke(&core.shared).await.is_ok(), acknowledged);
            assert_eq!(
                *calls.lock().unwrap(),
                vec![("owner.browserUnpair".into(), json!({"browser":"safari"}))]
            );
            core.shutdown().await;
        }
    }
}
