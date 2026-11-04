//! Chrome tool availability from the same state as the person's card. No setup actions run here.
//! The gateway and permission hook use these receipts; diagnoses keep the next step.
use super::{AddonState, AddonView, agent_browser::Browser};
use crate::core::Shared;
use serde_json::{Value, json};
use std::fmt::Write;

pub(super) fn reason(view: &AddonView, browser: Browser) -> String {
    let mut text = format!("Not run: {}", view.instruction);
    // Agent text only (the card keeps its instruction): an agent once started the person's Edge itself as a chat
    // background process, so Quit ended it with every tab the person had opened in it.
    if view.state == AddonState::ChromeClosed {
        text.push_str(&browser.text(
            " Do not start Chrome yourself: Vibe Work ends programs the agent started when it quits, and the person's tabs would close with it.",
        ));
    }
    if !matches!(
        (browser, view.state),
        (_, AddonState::Unavailable | AddonState::Connected)
            | (Browser::Edge, AddonState::ChromeClosed)
    ) {
        let _ = write!(
            text,
            " Offer the {} card with addon_suggest (addon: {}).",
            browser.text("Connect Chrome"),
            browser.id()
        );
    }
    text
}

/// Whether the helper reports a connected profile now: the owner's record of the helper's latest status answer
/// (polled every two seconds and on every call's own status read), not anything read from files.
pub(super) fn live(shared: &Shared, browser: Browser) -> bool {
    shared.chrome_control.owner.is_running() && browser.paired(shared)
}

/// The card's refusal (its diagnosis, the person's step and the `addon_suggest` hint) when Chrome cannot take a
/// call; `None` when the gateway's own checks decide. `live`: the helper reports a connected profile ([`live`], or
/// this call's own status read).
///
/// Invariants:
/// - Only the person's switch and an extension older than this app needs refuse a live connection: every other
///   non-Connected state means nothing is connected, or comes from files that can lag a live connection, so a
///   working Chrome is never refused for the card's lag.
/// - This never admits a call: the gateway's mode, lease, support and network checks run either way.
pub(super) fn refusal(shared: &Shared, browser: Browser, live: bool) -> Option<Value> {
    // Builds without Chrome keep their existing feature gate in the tool dispatcher.
    if !shared.chrome_control.config.available() {
        return None;
    }
    let view = browser.addon(shared);
    let blocks = match view.state {
        AddonState::Connected => false,
        AddonState::SwitchedOff | AddonState::UpdateNeeded => true,
        _ => !live,
    };
    if !blocks {
        return None;
    }
    if view.state == AddonState::Connecting && browser.reconnectable(shared) {
        return Some(super::reconnect::retry(browser));
    }
    let mut answer = super::owner::own(&reason(&view, browser), "OWNER_REQUIRED").into_value();
    answer["structuredContent"]["addon"] = json!({
        "state": view.label, "stateCode": view.state, "instruction": view.instruction,
        "extensionVersion":view.extension_version,
    });
    Some(json!({"vwMcpResult":answer}))
}

/// The card's refusal before a call is sent: [`refusal`], except while Chrome is connecting. The pairing was just
/// confirmed and the extension's connection is forming, so the helper's own answer decides; a failure it answers
/// gets the card's words afterwards (`gateway::call_one`). Seen in the Chrome e2e: right after Use my Chrome was
/// turned off and on, the card said Connecting and refused the chat's calls; with the helper deciding, they ran.
pub(super) fn gate(shared: &Shared, browser: Browser, live: bool) -> Option<Value> {
    if shared.chrome_control.config.available()
        && browser.addon(shared).state == AddonState::Connecting
    {
        return None;
    }
    refusal(shared, browser, live)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome_control::{gateway, owner, test_support};
    /// Each test names its own core: one folder per name and process, so parallel tests never share files.
    async fn core_with_min_version(name: &str) -> crate::Core {
        crate::offline_core::start_with(name, |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
            let root = dir.join("install");
            vw_vibe_install::extension_store::save_record(
                &root,
                Some(vw_vibe_install::support::ChromeExtension {
                    store: Some(vw_vibe_install::ExtensionStore::Available),
                    min_version: Some("0.1.6".into()),
                }),
            )
            .unwrap();
            config.vibe_install = Some(vw_vibe_install::VibeInstaller::new(
                vw_vibe_install::InstallerConfig {
                    root,
                    sources: vw_vibe_install::Sources::pinned(&vw_vibe_install::this_target()),
                    parent_env: Vec::new(),
                    extra_env: Vec::new(),
                    discovery: vw_supervisor::Discovery {
                        explicit: None,
                        path_var: None,
                        home: None,
                    },
                    limits: vw_vibe_install::Limits::default(),
                },
            ));
        })
        .await
    }

    #[test]
    fn a_closed_browser_tells_only_the_agent_not_to_start_it() {
        let view = crate::chrome_control::addon::view(
            AddonState::ChromeClosed,
            crate::chrome_control::setup::ABSENT,
        );
        assert!(reason(&view, Browser::Chrome).contains(
            "Do not start Chrome yourself: Vibe Work ends programs the agent started when it quits"
        ));
        assert!(reason(&view, Browser::Edge).contains("Do not start Edge yourself"));
        assert!(
            !view.instruction.contains("Do not start"),
            "the card's text stays as it was"
        );
    }

    #[tokio::test]
    async fn tools_report_the_cards_diagnosis_and_instruction() {
        let core = core_with_min_version("chrome-unavailable-tools").await;
        let shared = &core.shared;
        let _ = shared
            .store
            .load(
                json!({"session":{"id":"s"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"auto-approve"}),
        );
        let calls = test_support::answering(shared, |_, _| {
            panic!("unavailable tools must not reach Chrome")
        });
        let chrome = &shared.chrome_control;
        chrome
            .owner
            .tools
            .lock()
            .unwrap()
            .push(json!({"name":"chrome_status"}));
        let dir = chrome.dir().unwrap();
        let data = chrome
            .config
            .user_home
            .as_ref()
            .unwrap()
            .join("chrome-data");
        let profile = data.join("Profile 1");
        let extension = profile.join("Extensions").join(&chrome.config.extension_id);
        std::fs::write(
            dir.join("setup.json"),
            json!({"files":[],"browsers":["Google Chrome"],"userData":[data]}).to_string(),
        )
        .unwrap();
        for state in [
            AddonState::SwitchedOff,
            AddonState::ExtensionNotInstalled,
            AddonState::TurnedOffInChrome,
            AddonState::UpdateNeeded,
            AddonState::NotPaired,
        ] {
            chrome
                .set_enabled(state != AddonState::SwitchedOff)
                .unwrap();
            chrome.owner.pairing.lock().unwrap().0 = "Not connected".into();
            if state == AddonState::TurnedOffInChrome {
                std::fs::create_dir_all(&extension).unwrap();
                std::fs::write(profile.join("Preferences"), json!({"extensions":{"settings":{&chrome.config.extension_id:{"disable_reasons":[1]}}}}).to_string()).unwrap();
            }
            if state == AddonState::UpdateNeeded {
                std::fs::remove_file(profile.join("Preferences")).unwrap();
                std::fs::create_dir_all(extension.join("0.1.3_0")).unwrap();
                // An old connected extension is still too old for this app: the live profile reports its version.
                chrome.owner.pairing.lock().unwrap().0 = "Paired".into();
                chrome.owner.connected_profiles.observe(&json!({"profiles":[
                    {"profileId":"p","connected":true,"extensionVersion":"0.1.3"}]}));
            }
            if state == AddonState::NotPaired {
                owner::observe(&std::sync::Arc::downgrade(shared), &json!({"method":"notifications/owner/pairing", "params":{"profileId":"chrome:p","state":"waiting","fingerprint":"fixture"}})).unwrap();
            }
            let card = crate::chrome_control::commands::addon(shared);
            assert_eq!(card.state, state);
            for (tool, args) in [
                ("chrome_status", json!({})),
                ("chrome_session", json!({"action":"open","profileId":"p"})),
                (
                    "chrome_execute",
                    json!({"sessionId":"old","action":"page.read","args":{}}),
                ),
            ] {
                let reply = gateway::call(shared, "s", tool, args).await.unwrap();
                let text = reply["vwMcpResult"]["content"][0]["text"].as_str().unwrap();
                assert!(
                    text.contains(&card.instruction) && text.contains("addon_suggest"),
                    "{state:?} {tool}: {text}"
                );
                assert_eq!(
                    reply["vwMcpResult"]["structuredContent"]["addon"]["stateCode"],
                    json!(state)
                );
            }
        }
        assert!(calls.lock().unwrap().is_empty());
        core.shutdown().await;
    }
    /// An extension older than this app needs keeps refusing the new actions: through the live feature gate while the
    /// card does not know the connected profile's version yet, and with the card's words once it reports it.
    #[tokio::test]
    async fn an_outdated_extension_keeps_refusing_the_new_actions() {
        let core = core_with_min_version("chrome-outdated-new-actions").await;
        let shared = &core.shared;
        let _ = shared
            .store
            .load(
                json!({"session":{"id":"s"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"auto-approve"}),
        );
        let chrome = &shared.chrome_control;
        chrome.tasks.lock().unwrap().insert("t".into(), "s".into());
        let old = json!({"profileId":"p","connected":true,"extensionVersion":"0.1.3",
            "features":["page.read"],"extensionCapabilities":["page.read"]});
        let status = json!({"structuredContent":{"sessions":[{"sessionId":"t","profileId":"p"}],"profiles":[old]}});
        let seen = test_support::answering(shared, move |tool, _| {
            if tool == "chrome_status" {
                status.clone()
            } else {
                json!({"structuredContent":{"status":"completed"}})
            }
        });
        let args = json!({"sessionId":"t","action":"clipboard.read","args":{}});
        assert_eq!(
            crate::chrome_control::commands::addon(shared).state,
            AddonState::Connected
        );
        let reason = gateway::call(shared, "s", "chrome_execute", args.clone())
            .await
            .unwrap_err();
        assert!(reason.contains("does not report support"), "{reason}");
        chrome
            .owner
            .connected_profiles
            .observe(&json!({"profiles":[old]}));
        assert_eq!(
            crate::chrome_control::commands::addon(shared).state,
            AddonState::UpdateNeeded
        );
        let reply = gateway::call(shared, "s", "chrome_execute", args)
            .await
            .unwrap();
        assert_eq!(
            reply["vwMcpResult"]["structuredContent"]["addon"]["stateCode"],
            json!(AddonState::UpdateNeeded),
            "{reply}"
        );
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .all(|(name, _)| name == "chrome_status")
        );
        core.shutdown().await;
    }
}
