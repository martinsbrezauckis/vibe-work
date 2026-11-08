//! Profile choice regressions through the real views and gateway, with isolated Chrome folders and fake helpers.
//! Disabled external copies cannot replace a paired profile or block its launch.
use super::{AddonState, agent_browser::Browser, gateway, installation, owner, test_support};
use serde_json::{Value, json};
use std::sync::Arc;

fn preferences(profile: &std::path::Path, id: &str, reasons: &Value) -> std::io::Result<()> {
    std::fs::write(
        profile.join("Preferences"),
        json!({"extensions":{"settings":{
            id:{"disable_reasons":reasons}
        }}})
        .to_string(),
    )
}

#[tokio::test]
async fn disabled_external_copies_do_not_block_the_paired_browser_or_its_launch() {
    for browser in [Browser::Chrome, Browser::Edge] {
        let core = super::browser_start_tests::fixture(
            &format!("browser-disabled-unpaired-{}", browser.id()),
            browser,
            Arc::new(|_| Ok(())),
        )
        .await;
        let shared = &core.shared;
        let chrome = &shared.chrome_control;
        let data = chrome
            .config
            .user_home
            .as_ref()
            .unwrap()
            .join("browser-data");
        let pending = data.join("Profile 2");
        std::fs::create_dir_all(
            pending
                .join("Extensions")
                .join(&chrome.config.extension_id)
                .join("0.0.1_0"),
        )
        .unwrap();
        for reasons in [json!(8192), json!([8192]), json!([1])] {
            preferences(&pending, &chrome.config.extension_id, &reasons).unwrap();
            assert_eq!(browser.addon(shared).state, AddonState::ChromeClosed);
            assert_eq!(
                browser.addon(shared).extension_version.as_deref(),
                Some("0.1.6")
            );
            assert_eq!(
                installation::paired_directory(shared, browser).as_deref(),
                Some("Default")
            );
        }
        // A missing launch record can be learned while a healthy profile and a disabled copy coexist.
        installation::forget_directory(chrome, browser);
        let profile = format!("{}:p", browser.id());
        let connected =
            json!({"profiles":[{"profileId":profile,"connected":true,"extensionVersion":"0.1.6"}]});
        test_support::observe_status(shared, &connected);
        assert_eq!(browser.addon(shared).state, AddonState::Connected);
        assert_eq!(
            installation::paired_directory(shared, browser).as_deref(),
            Some("Default")
        );
        test_support::observe_status(shared, &json!({"profiles":[]}));
        // The paired copy's own disable reason must still give the person the enable/turn-on step.
        for (reasons, state) in [
            (json!(8192), AddonState::EnableInChrome),
            (json!([1]), AddonState::TurnedOffInChrome),
        ] {
            preferences(&data.join("Default"), &chrome.config.extension_id, &reasons).unwrap();
            assert_eq!(browser.addon(shared).state, state);
        }
        preferences(
            &data.join("Default"),
            &chrome.config.extension_id,
            &json!(0),
        )
        .unwrap();
        let weak = Arc::downgrade(shared);
        let seen = chrome.owner.answering(move |tool, args| {
            let answer = if tool == "chrome_status" {
                json!({"structuredContent":connected})
            } else {
                assert_eq!(args["profileId"], profile);
                json!({"structuredContent":{"sessionId":"task-paired","profileId":profile,"state":"active"}})
            };
            owner::update_pairing(&weak, &answer);
            answer
        });
        let reply = gateway::call(
            shared,
            "a",
            "chrome_session",
            json!({"action":"open","profileId":browser.id()}),
        )
        .await
        .unwrap();
        assert_eq!(
            reply["vwMcpResult"]["structuredContent"]["sessionId"], "task-paired",
            "{reply}"
        );
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .filter(|(tool, _)| tool == "chrome_session")
                .count(),
            1
        );
        // A second enabled profile is still a real choice.
        preferences(&pending, &chrome.config.extension_id, &json!(0)).unwrap();
        assert!(installation::paired_directory(shared, browser).is_none());
        core.shutdown().await;
    }
}

#[tokio::test]
async fn disabled_paired_copies_count_until_their_folder_is_deleted() {
    for browser in [Browser::Chrome, Browser::Edge] {
        let core = super::browser_start_tests::fixture(
            &format!("browser-disabled-paired-{}", browser.id()),
            browser,
            Arc::new(|_| Ok(())),
        )
        .await;
        let shared = &core.shared;
        let chrome = &shared.chrome_control;
        let data = chrome
            .config
            .user_home
            .as_ref()
            .unwrap()
            .join("browser-data");
        let pending = data.join("Profile 2");
        std::fs::create_dir_all(pending.join("Extensions").join(&chrome.config.extension_id))
            .unwrap();
        preferences(&pending, &chrome.config.extension_id, &json!(0)).unwrap();
        let second = format!("{}:second", browser.id());
        test_support::observe_status(
            shared,
            &json!({"profiles":[
                {"profileId":format!("{}:p", browser.id()),"connected":true,"extensionVersion":"0.1.6"},
                {"profileId":second,"connected":true,"extensionVersion":"0.1.6"}
            ]}),
        );
        preferences(&pending, &chrome.config.extension_id, &json!(8192)).unwrap();
        test_support::observe_status(shared, &json!({"profiles":[]}));
        assert!(
            installation::paired_directory(shared, browser).is_none(),
            "a disabled paired copy is still a profile choice"
        );
        std::fs::remove_dir_all(&pending).unwrap();
        assert_eq!(
            installation::paired_directory(shared, browser).as_deref(),
            Some("Default")
        );
        core.shutdown().await;
    }
}
