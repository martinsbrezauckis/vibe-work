//! The gateway wakes only the selected browser, once per lost connection, before dispatching any action.
//! Uses injected browser launchers and real owner/gateway state; no desktop or native host is required.
use super::super::{agent_browser::Browser, edge_agent_tests::fixture_with, gateway, owner};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn a_stopped_worker_is_woken_before_the_action_is_sent_once() {
    for (browser, profile) in [(Browser::Chrome, "chrome:p"), (Browser::Edge, "edge:p")] {
        let launches = Arc::new(Mutex::new(Vec::new()));
        let recorded = launches.clone();
        let core = fixture_with(
            &format!("browser-worker-wake-{profile}"),
            json!([{"profileId":profile,"connected":true}]),
            move |config| {
                let launch = Arc::new(move |url: &str| {
                    recorded.lock().unwrap().push(url.to_owned());
                    Ok(())
                });
                if browser == Browser::Edge {
                    config.edge_launcher = Some(launch);
                } else {
                    config.launcher = Some(launch);
                }
            },
        )
        .await;
        gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":profile}),
        )
        .await
        .unwrap();
        let chrome = &core.shared.chrome_control;
        if browser == Browser::Edge {
            chrome.owner.edge_pairing.lock().unwrap().0 =
                crate::chrome_control::PairingState::Connecting;
        } else {
            chrome.owner.pairing.lock().unwrap().0 =
                crate::chrome_control::PairingState::Connecting;
        }
        let weak = Arc::downgrade(&core.shared);
        let observed = launches.clone();
        let seen = chrome.owner.answering(move |tool, _| {
            let result = if tool == "chrome_status" {
                json!({"structuredContent":{"profiles":[{"profileId":profile,"connected":!observed.lock().unwrap().is_empty()}]}})
            } else { json!({"structuredContent":{"status":"completed"}}) };
            owner::update_pairing(&weak, &result);
            result
        });
        let reply = gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"task","action":"page.read","args":{}}),
        )
        .await
        .unwrap();
        assert_eq!(
            reply["vwMcpResult"]["structuredContent"]["status"], "completed",
            "{reply}"
        );
        assert_eq!(
            *launches.lock().unwrap(),
            [format!(
                "chrome-extension://{}/ui.html?surface=panel",
                chrome.config.extension_id
            )]
        );
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .filter(|(name, _)| name == "chrome_execute")
                .count(),
            1
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn failed_wakes_do_not_open_another_tab_on_each_call() {
    let launches = Arc::new(Mutex::new(Vec::new()));
    let recorded = launches.clone();
    let admission = Arc::new(tokio::sync::Notify::new());
    let ready = admission.clone();
    let core = fixture_with(
        "browser-worker-stuck",
        json!([{"profileId":"edge:p","connected":true}]),
        move |config| {
            config.edge_launcher = Some(Arc::new(move |url| {
                recorded.lock().unwrap().push(url.to_owned());
                ready.notify_one();
                Ok(())
            }));
        },
    )
    .await;
    core.shared
        .chrome_control
        .owner
        .edge_pairing
        .lock()
        .unwrap()
        .0 = crate::chrome_control::PairingState::Connecting;
    core.shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"structuredContent":{"profiles":[]}}));
    let mut paused = false;
    for _ in 0..2 {
        let shared = core.shared.clone();
        let mut call = tokio::spawn(async move {
            gateway::call(
                &shared,
                "s",
                "chrome_session",
                json!({"action":"open","profileId":"edge:p"}),
            )
            .await
        });
        // The detached launcher uses an OS worker. Observe its actual call before advancing virtual
        // reconnect time; a busy worker/executor must not count against this policy assertion.
        let completed = if paused {
            None
        } else {
            let completed = tokio::select! {
                () = admission.notified() => None,
                result = &mut call => Some(result),
            };
            tokio::time::pause();
            paused = true;
            completed
        };
        let started = tokio::time::Instant::now();
        let reply = match completed {
            Some(result) => result,
            None => call.await,
        }
        .unwrap()
        .unwrap();
        assert_eq!(
            reply["vwMcpResult"]["structuredContent"]["error"]["code"],
            "EXTENSION_UNRESPONSIVE"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(7));
    }
    assert_eq!(
        launches.lock().unwrap().len(),
        1,
        "at most one wake per disconnected browser"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn macos_remembered_pairing_can_wake_without_a_profile_folder_probe() {
    let forbidden = Arc::new(Mutex::new(Vec::new()));
    let attempts = forbidden.clone();
    let core = fixture_with(
        "mac-remembered-wake",
        json!([{"profileId":"edge:p","connected":true}]),
        |config| {
            config.macos = Some(true);
            config.file_access = Some(Arc::new(move |_, path| {
                if path.to_string_lossy().contains("Microsoft Edge") {
                    attempts.lock().unwrap().push(path.to_owned());
                    return Err(std::io::Error::from_raw_os_error(1));
                }
                Ok(())
            }));
        },
    )
    .await;
    core.shared.chrome_control.owner.connected_profiles.clear();
    assert!(
        super::known_extension(&core.shared, Browser::Edge),
        "remembered pairing must allow a wake without a readable profile folder"
    );
    assert!(!super::known_extension(&core.shared, Browser::Chrome));
    assert!(forbidden.lock().unwrap().is_empty());
    core.shutdown().await;
}
