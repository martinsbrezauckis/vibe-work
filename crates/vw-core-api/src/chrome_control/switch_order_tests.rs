//! The local-network switch reaches the helper in the order the person set it: a delivery that read an older value
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
async fn an_older_on_delivery_held_past_an_off_acknowledgement_never_becomes_the_rule() {
    let core = core_with_chrome("switch-order").await;
    let shared = core.shared.clone();
    let chrome = &shared.chrome_control;
    let revision = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let next = revision.clone();
    let seen = chrome.owner.answering(move |_, _| {
        let revision = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        json!({"structuredContent":{"revision":revision,"pendingProfiles":["p"]}})
    });
    chrome.set_local_network(true).unwrap();
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
    chrome.set_local_network(false).unwrap();
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
    assert_eq!(
        taken.last(),
        Some(&false),
        "the helper's rule is the person's current switch: {taken:?}"
    );
    let first_off = taken
        .iter()
        .position(|on| !on)
        .expect("off reached the helper");
    assert!(
        taken[first_off..].iter().all(|on| !on),
        "no older on after the off: {taken:?}"
    );
    assert!(chrome.local_network_applied(), "off is applied");
    core.shutdown().await;
}
