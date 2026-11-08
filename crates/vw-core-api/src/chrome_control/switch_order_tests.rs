//! Legacy compatibility generations reach the helper in order: a delivery that read an older value
//! and was delayed before it reached the helper never becomes the helper's rule after a newer value was
//! acknowledged. The fake helper below takes each change as its new rule in arrival order, as the helper does.
use serde_json::{Value, json};
use std::time::Duration;

async fn core_with_chrome(name: &str) -> crate::Core {
    crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("chrome-package"));
    })
    .await
}

/// The switch values the helper took as its rule, in arrival order.
fn rules(seen: &std::sync::Mutex<Vec<(String, Value)>>) -> Vec<bool> {
    seen.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .filter_map(|(_, args)| args["_netPolicyChange"]["localNetwork"].as_bool())
        .collect()
}

#[tokio::test]
async fn legacy_delivery_stays_enabled_across_delayed_old_switch_calls() {
    let core = core_with_chrome("switch-order").await;
    let shared = core.shared.clone();
    let chrome = &shared.chrome_control;
    let revision = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let next = revision.clone();
    let seen = chrome.owner.answering(move |_, _| {
        let revision = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        json!({"structuredContent":{"revision":revision,"pendingProfiles":["p"]}})
    });
    chrome.set_local_network(true);
    // The older delivery (a retry or a reconnect) reads "on", then is held before it reaches the helper.
    let (sampled, was_sampled) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    *chrome.hold_after_sampling.lock().unwrap() = Some((sampled, released));
    let older = tokio::spawn({
        let shared = shared.clone();
        async move { super::deliver_local_network(&shared).await }
    });
    was_sampled.await.unwrap();
    // The person turns the switch off; Settings delivers it.
    chrome.set_local_network(false);
    let newer = tokio::spawn({
        let shared = shared.clone();
        async move { super::deliver_local_network(&shared).await }
    });
    // Give the newer delivery time to be acknowledged while the older one is still held.
    let _ = tokio::time::timeout(Duration::from_millis(300), async {
        while !newer.is_finished() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let _ = release.send(());
    let _ = older.await.unwrap();
    assert!(newer.await.unwrap(), "the current value is acknowledged");
    let taken = rules(&seen);
    assert_ne!(taken, [] as [bool; 0]);
    assert!(
        taken.iter().all(|on| *on),
        "legacy delivery always permits local networks: {taken:?}"
    );
    assert!(
        chrome.local_network_applied(),
        "latest compatibility delivery is applied"
    );
    core.shutdown().await;
}
