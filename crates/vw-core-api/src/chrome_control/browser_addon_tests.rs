//! Browser-local installation facts through Settings and the agent gateway, using real preference fixtures
//! and the existing fake helper. Neither browser's dormant profiles may describe the other browser.
use super::PairingState;
use super::{AddonState, agent_browser::Browser, commands, edge_agent_tests::fixture, gateway};
use serde_json::json;

#[tokio::test]
async fn broken_helper_keeps_the_reinstall_reason_for_both_browsers() {
    use vw_vocabulary::texts::{CHROME_PACKAGE_CHANGED, CHROME_PACKAGE_MISSING};
    for (name, bytes, reason) in [
        ("missing", None, CHROME_PACKAGE_MISSING),
        (
            "changed",
            Some(b"changed helper".as_slice()),
            CHROME_PACKAGE_CHANGED,
        ),
    ] {
        let core =
            crate::offline_core::start_with(&format!("browser-helper-{name}"), |config, dir| {
                config.chrome_control.package = Some(dir.join("package"));
                config.chrome_control.helper_sha256 = Some("0".repeat(64));
            })
            .await;
        let chrome = &core.shared.chrome_control;
        let package = chrome.config.package.as_ref().unwrap();
        let helper = package.join("bin").join(super::setup::helper_name());
        if let Some(bytes) = bytes {
            std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
            std::fs::write(&helper, bytes).unwrap();
        } else if helper.exists() {
            std::fs::remove_file(&helper).unwrap();
        }
        // Neither an empty pairing report nor a stale successful one may hide the package diagnosis.
        for pairing in [PairingState::Unknown, PairingState::Paired] {
            chrome.owner.pairing.lock().unwrap().0 = pairing;
            chrome.owner.edge_pairing.lock().unwrap().0 = pairing;
            let view = commands::view(&core.shared);
            let edge = view.edge.unwrap();
            assert!(!view.setup.package && !edge.package);
            assert_eq!(view.setup.pairing, reason);
            assert_eq!(edge.pairing, reason);
            let addon = edge.addon.unwrap();
            assert_eq!(addon.state, AddonState::Unavailable);
            assert_eq!(addon.action, None);
            assert_eq!(addon.label, Browser::Edge.text(reason));
        }
        core.shutdown().await;
    }
}

#[allow(
    clippy::unwrap_used,
    clippy::too_many_lines,
    reason = "test-only fixture and card/agent assertions must fail on an error"
)]
async fn disconnected(name: &str, chrome_version: Option<&str>, chrome_off: bool, edge_off: bool) {
    let core = fixture(name, json!([])).await;
    let chrome = &core.shared.chrome_control;
    let home = chrome.config.user_home.as_ref().unwrap();
    let data = [home.join("chrome-data"), home.join("edge-data")];
    for ((dir, version), off) in data
        .iter()
        .zip([chrome_version, Some("0.1.4")])
        .zip([chrome_off, edge_off])
    {
        let _ = std::fs::remove_dir_all(dir);
        let profile = dir.join("Profile 1");
        std::fs::create_dir_all(&profile).unwrap();
        let Some(version) = version else { continue };
        std::fs::create_dir_all(
            profile
                .join("Extensions")
                .join(&chrome.config.extension_id)
                .join(format!("{version}_0")),
        )
        .unwrap();
        std::fs::write(profile.join("Secure Preferences"), json!({"extensions":{"settings":{&chrome.config.extension_id:{"disable_reasons":if off {json!([2])} else {json!([])}}}}}).to_string()).unwrap();
    }
    std::fs::write(
        chrome.dir().unwrap().join("setup.json"),
        json!({"files":[],"browsers":["Google Chrome","Microsoft Edge"],"userData":data})
            .to_string(),
    )
    .unwrap();
    chrome.owner.pairing.lock().unwrap().0 = crate::chrome_control::PairingState::Disconnected;
    chrome.owner.edge_pairing.lock().unwrap().0 = crate::chrome_control::PairingState::Disconnected;
    chrome
        .owner
        .tools
        .lock()
        .unwrap()
        .push(json!({"name":"chrome_status"}));
    chrome
        .edge_paired_once
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // Read twice: a positive cache from Chrome must never contaminate Edge, or vice versa.
    for _ in 0..2 {
        let view = commands::view(&core.shared);
        let c = view.setup.addon.unwrap();
        let e = view.edge.unwrap().addon.unwrap();
        assert_eq!(
            c.state,
            if chrome_off {
                AddonState::TurnedOffInChrome
            } else if chrome_version.is_none() {
                AddonState::ExtensionNotInstalled
            } else {
                AddonState::NotConnected
            }
        );
        assert_eq!(
            e.state,
            if edge_off {
                AddonState::TurnedOffInChrome
            } else {
                AddonState::ChromeClosed
            }
        );
        assert_eq!(
            serde_json::to_value(&c).unwrap()["extensionVersion"],
            json!(chrome_version)
        );
        assert_eq!(
            serde_json::to_value(&e).unwrap()["extensionVersion"],
            "0.1.4"
        );
        if edge_off {
            assert_eq!(e.label, "Turn on in Edge");
            assert!(e.instruction.contains("turned off in Edge"));
            assert!(!e.instruction.contains("Chrome") && !e.instruction.contains("reconnect"));
            let refusal = super::availability::refusal(&core.shared, Browser::Edge, false).unwrap();
            let text = refusal["vwMcpResult"]["content"][0]["text"]
                .as_str()
                .unwrap();
            assert!(
                text.contains("Connect Edge") && text.contains("addon: edge"),
                "{text}"
            );
            assert_eq!(
                refusal["vwMcpResult"]["structuredContent"]["addon"]["extensionVersion"],
                "0.1.4"
            );
        }
    }
    if edge_off {
        if chrome_version.is_none() {
            // Removing Chrome's extension need not erase its historical pairing marker.
            chrome
                .paired_once
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let status = gateway::call(&core.shared, "s", "chrome_status", json!({}))
            .await
            .unwrap();
        assert_eq!(
            status["vwMcpResult"]["structuredContent"]["addon"]["stateCode"], "turnedOffInChrome",
            "{status}"
        );
        assert!(
            status["vwMcpResult"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("turned off in Edge")
        );
        let answer = gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"edge:p"}),
        )
        .await
        .unwrap();
        assert_eq!(
            answer["vwMcpResult"]["structuredContent"]["addon"]["stateCode"],
            "turnedOffInChrome"
        );
        chrome.set_enabled(false).unwrap();
        assert_eq!(
            Browser::Edge.addon(&core.shared).state,
            AddonState::SwitchedOff
        );
        chrome.set_enabled(true).unwrap();
        chrome.owner.edge_pairing.lock().unwrap().0 = crate::chrome_control::PairingState::Paired;
        assert_eq!(
            Browser::Edge.addon(&core.shared).state,
            AddonState::Connected
        );
    }
    core.shutdown().await;
}

#[tokio::test]
async fn version_prefers_the_paired_profile_else_the_highest_in_this_browser() {
    let core = fixture(
        "browser-version-selection",
        json!([
            {"profileId":"chrome:p","connected":true,"extensionVersion":"0.1.3"}
        ]),
    )
    .await;
    let chrome = &core.shared.chrome_control;
    let data = chrome.config.user_home.as_ref().unwrap().join("versions");
    for (name, version) in [("Profile 1", "0.1.3"), ("Profile 2", "0.1.9")] {
        std::fs::create_dir_all(
            data.join(name)
                .join("Extensions")
                .join(&chrome.config.extension_id)
                .join(format!("{version}_0")),
        )
        .unwrap();
    }
    std::fs::write(
        chrome.dir().unwrap().join("setup.json"),
        json!({"files":[],"browsers":["Google Chrome","Microsoft Edge"],"userData":[data]})
            .to_string(),
    )
    .unwrap();
    assert_eq!(
        super::installation::version(&core.shared, Browser::Chrome).as_deref(),
        Some("0.1.3")
    );
    chrome.owner.connected_profiles.clear();
    assert_eq!(
        super::installation::version(&core.shared, Browser::Chrome).as_deref(),
        Some("0.1.9")
    );
    assert_eq!(
        super::installation::version(&core.shared, Browser::Edge),
        None
    );
    core.shutdown().await;
}

#[tokio::test]
async fn only_edge_installed_and_turned_off() {
    disconnected("only-edge-off", None, false, true).await;
}

#[tokio::test]
async fn chrome_turned_off_and_edge_enabled() {
    disconnected("only-chrome-off", Some("0.1.3"), true, false).await;
}

#[tokio::test]
async fn both_browsers_turned_off_keep_their_own_versions() {
    disconnected("both-browsers-off", Some("0.1.3"), true, true).await;
}

#[tokio::test]
async fn status_reports_both_browser_versions_even_when_one_is_not_selected() {
    let core = fixture(
        "browser-status-versions",
        json!([
            {"profileId":"chrome:p","connected":true,"extensionVersion":"0.1.3"},
            {"profileId":"edge:p","connected":true,"extensionVersion":"0.1.4"}
        ]),
    )
    .await;
    let answer = gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    let body = &answer["vwMcpResult"]["structuredContent"];
    let selected = body.get("untrustedPageContent").unwrap_or(body);
    assert_eq!(selected["addon"]["extensionVersion"], "0.1.3");
    assert_eq!(body["addons"][0]["stateCode"], "connected");
    assert_eq!(body["addons"][0]["extensionVersion"], "0.1.3");
    assert_eq!(body["addons"][1]["stateCode"], "connected");
    assert_eq!(body["addons"][1]["extensionVersion"], "0.1.4");
    core.shutdown().await;
}

#[tokio::test]
async fn reworded_pairing_labels_keep_agent_diagnoses_and_actions() {
    let core = fixture("browser-reworded-pairing", json!([])).await;
    for (pairing, expected) in [
        (PairingState::Paired, AddonState::Connected),
        (PairingState::OtherOwner, AddonState::OtherOwner),
        (PairingState::Waiting, AddonState::NotPaired),
    ] {
        core.shared
            .chrome_control
            .owner
            .edge_pairing
            .lock()
            .unwrap()
            .0 = pairing;
        let mut setup = *commands::view(&core.shared).edge.unwrap();
        let original = super::agent_browser::edge_addon(&core.shared, &setup);
        assert_eq!(original.state, expected);
        setup.pairing = "Completely different display words".into();
        let reworded = super::agent_browser::edge_addon(&core.shared, &setup);
        assert_eq!(original, reworded);
    }
    let dynamic = "Could not read a page titled Google Chrome in Chrome";
    assert_eq!(Browser::Edge.text(dynamic), dynamic);
    core.shutdown().await;
}

#[test]
fn not_set_up_instructions_name_each_browsers_card_button() {
    for (browser, button) in [
        (Browser::Chrome, "Connect Chrome"),
        (Browser::Edge, "Set up…"),
    ] {
        let view = super::addon::view_for(AddonState::NotSetUp, "", browser);
        let location = if browser == Browser::Edge {
            format!(" in Settings, Browser, {}", browser.name())
        } else {
            String::new()
        };
        assert_eq!(
            view.instruction,
            format!(
                "{} is not set up. Ask the person to click {button}{location}.",
                browser.words("{yourBrowser}")
            )
        );
        let reason = super::availability::reason(&view, browser);
        assert!(
            reason.contains(&format!("click {button}{location}.")),
            "{reason}"
        );
        if browser == Browser::Edge {
            assert!(!reason.contains("Connect Edge"), "{reason}");
        }
    }
}
