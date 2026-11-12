//! Read-only session facts, add-on states and connector/add-on suggestions for the agent. Uses core mode state, the
//! add-on state model (`chrome_control::addon`) and Vibe's native catalog. Called by the session tool dispatcher; no
//! path installs, enables, pairs or authenticates anything. The add-on states are reported whatever the chat's
//! launch line (a chat without the Chrome tools still learns why and what the person does).

use crate::chrome_control::agent_browser::Browser;

fn browsers() -> impl Iterator<Item = Browser> {
    [Browser::Chrome]
        .into_iter()
        .chain(cfg!(feature = "your-edge").then_some(Browser::Edge))
}

/// Every shipped add-on reports its own state, even when this chat has no browser tools.
fn addons(shared: &Shared) -> Value {
    crate::chrome_control::installation::addons(shared)
}

fn reason(args: &Value) -> Result<&str, String> {
    args.get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|reason| !reason.is_empty() && reason.chars().count() <= 500)
        .ok_or_else(|| "reason must contain 1 to 500 characters".to_owned())
}

/// `addon_suggest`: the add-on's own control as a card for the person. Refused, with the reason and what to do,
/// when there is nothing for the person to set up.
fn addon_suggest(shared: &Shared, args: &Value) -> Result<Value, String> {
    let addon = args
        .get("addon")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let browser = browsers()
        .find(|browser| browser.id() == addon)
        .ok_or("Not run: unknown add-on. Use session_status to list the available add-ons.")?;
    let reason = reason(args)?;
    let view = browser.addon(shared);
    match view.state {
        crate::chrome_control::AddonState::Connecting
        | crate::chrome_control::AddonState::ChromeClosed
            if browser.reconnectable(shared) =>
        {
            Err(format!("Not shown: {}", view.instruction))
        }
        crate::chrome_control::AddonState::Connected => Err(format!(
            "Not shown: {} is already connected; nothing to set up. Use the {} tools, or start a new chat if this one has none.",
            browser.name(),
            browser.text("Chrome")
        )),
        crate::chrome_control::AddonState::Unavailable => {
            Err(format!("Not shown: {}", view.instruction))
        }
        _ => Ok(json!({"addonSuggestion":{
            "addon": browser.id(),
            "name": browser.name(),
            "reason": reason,
            "state": view.label,
            "stateCode": view.state,
            "label": view.label,
            "instruction": view.instruction,
            "extensionVersion": view.extension_version,
        }})),
    }
}
use crate::{Core, Origin, core::Shared};
use serde_json::{Value, json};
use std::sync::Arc;
use vw_agent_tools::wire::tools;

pub(super) async fn call(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let view = shared
        .store
        .view(session)
        .ok_or("this session is not open in Vibe Work right now")?;
    let core = Core {
        shared: shared.clone(),
    };
    if tool == tools::SESSION_STATUS {
        let mode = shared.access.remembered(session);
        let label = mode
            .as_deref()
            .map(|mode| vw_vocabulary::label("accessModes", mode));
        return Ok(json!({
            "accessMode":mode,
            "accessModeName":label,
            "model":view.state["session"]["model"],
            "thinkingLevel":view.state["session"]["reasoningEffort"],
            "vibeVersion":core.status().vibe_version,
            "chromeExtensionStore":crate::chrome_control::commands::extension_store(shared),
            "addons":addons(shared),
            "network":crate::network_facts::read(shared).await,
        }));
    }
    if tool == tools::ADDON_SUGGEST {
        return addon_suggest(shared, args);
    }
    // The catalog's transport future is large; keep the per-tool socket task's future bounded.
    let view = Box::pin(core.connectors_view(Origin::Ui, Some(session.to_owned()))).await;
    if let Some(error) = view.catalog_error {
        return Err(format!(
            "Mistral's connector catalog could not be read: {error}"
        ));
    }
    if tool == tools::CONNECTORS_LIST {
        return Ok(json!({"connectors":view.connectors}));
    }
    let alias = args
        .get("alias")
        .and_then(Value::as_str)
        .ok_or("alias is required")?;
    let reason = reason(args)?;
    let connector = view
        .connectors
        .iter()
        .find(|connector| connector.alias == alias)
        .ok_or("This connector is not in Mistral's available catalog")?;
    if connector.readiness == "unavailable" {
        return Err("This connector is unavailable for this account".into());
    }
    Ok(
        json!({"connectorSuggestion":{"alias":connector.alias,"name":connector.display_name,"reason":reason}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use vw_vibe_install::{ExtensionStore, InstallerConfig, Limits, Sources, VibeInstaller};

    #[tokio::test]
    async fn session_status_reports_this_sessions_model_and_thinking_without_inventing_defaults() {
        let core = crate::offline_core::start("session-model-facts").await;
        for (id, model, thinking) in [
            ("known", json!("my-model"), json!("high")),
            ("unknown", Value::Null, Value::Null),
            ("off", json!("other-model"), json!("off")),
        ] {
            let _ = core.shared.store.load(json!({"session":{"id":id,"cwd":"/project","model":model,"reasoningEffort":thinking,"status":{"type":"idle"}},"entries":[],"turns":[]}), vw_session::LoadKind::New).unwrap();
            let status = call(&core.shared, id, tools::SESSION_STATUS, &json!({}))
                .await
                .unwrap();
            assert_eq!(status.get("model"), Some(&model), "{status}");
            assert_eq!(status.get("thinkingLevel"), Some(&thinking), "{status}");
        }
        core.shutdown().await;
    }

    #[tokio::test]
    async fn session_status_reports_the_recorded_extension_store_without_a_chrome_package() {
        for (index, store) in [
            Some(ExtensionStore::Review),
            Some(ExtensionStore::Available),
            None,
        ]
        .into_iter()
        .enumerate()
        {
            let core = crate::offline_core::start_with(
                &format!("session-extension-state-{index}"),
                |config, dir| {
                    let root = dir.join("install");
                    vw_vibe_install::extension_store::save(&root, store).unwrap();
                    config.vibe_install = Some(VibeInstaller::new(InstallerConfig {
                        root,
                        sources: Sources::pinned(&vw_vibe_install::this_target()),
                        parent_env: Vec::new(),
                        extra_env: Vec::new(),
                        discovery: vw_supervisor::Discovery {
                            explicit: None,
                            path_var: None,
                            home: None,
                        },
                        limits: Limits::default(),
                    }));
                },
            )
            .await;
            let _ = core.shared.store.load(json!({"session":{"id":"s","cwd":"/project","status":{"type":"idle"}},"entries":[],"turns":[]}), vw_session::LoadKind::New).unwrap();
            let status = call(&core.shared, "s", tools::SESSION_STATUS, &json!({}))
                .await
                .unwrap();
            core.shutdown().await;
            assert_eq!(
                status.get("chromeExtensionStore"),
                Some(&json!(store)),
                "{status}"
            );
            assert_eq!(
                status.get("vibeVersion"),
                Some(&Value::Null),
                "no running server is not a pinned version"
            );
        }
    }

    async fn chat(name: &str, package: bool) -> crate::Core {
        let core = crate::offline_core::start_with(name, move |config, dir| {
            if package {
                let package = dir.join("package");
                std::fs::create_dir_all(package.join("bin")).unwrap();
                std::fs::write(
                    package.join("bin").join(if cfg!(windows) {
                        "chrome-control.exe"
                    } else {
                        "chrome-control"
                    }),
                    b"test helper",
                )
                .unwrap();
                config.chrome_control.package = Some(package);
                config.chrome_control.user_home = Some(dir.join("home"));
                config.chrome_control.launcher = Some(std::sync::Arc::new(|_| Ok(())));
            }
        })
        .await;
        let _ = core.shared.store.load(json!({"session":{"id":"s","cwd":"/project","status":{"type":"idle"}},"entries":[],"turns":[]}), vw_session::LoadKind::New).unwrap();
        core
    }

    /// A chat launched without the Chrome tools (no package) still learns Your Chrome's state and what to do.
    #[tokio::test]
    async fn session_status_reports_each_addon_with_an_instruction() {
        for (name, package, state, label) in [
            ("session-addon-none", false, "unavailable", None),
            (
                "session-addon-setup",
                true,
                "notSetUp",
                Some("Not connected"),
            ),
        ] {
            let core = chat(name, package).await;
            let status = call(&core.shared, "s", tools::SESSION_STATUS, &json!({}))
                .await
                .unwrap();
            core.shutdown().await;
            let chrome = &status["addons"][0];
            assert_eq!(chrome["id"], "chrome", "{status}");
            assert_eq!(
                chrome["name"],
                vw_vocabulary::texts::CHROME_NAME,
                "{status}"
            );
            assert_eq!(chrome["stateCode"], state, "{status}");
            assert_ne!(chrome["state"], state, "person words: {status}");
            if let Some(label) = label {
                assert_eq!(chrome["label"], label, "{status}");
            }
            let instruction = chrome["instruction"].as_str().unwrap();
            assert!(
                instruction.to_ascii_lowercase().contains("ask the user"),
                "{instruction}"
            );
        }
    }

    #[cfg(feature = "your-edge")]
    #[tokio::test]
    async fn paired_reconnect_does_not_offer_an_owner_card_for_either_browser() {
        for (addon, profile) in [("chrome", "chrome:p"), ("edge", "edge:p")] {
            let core = chat(&format!("session-addon-reconnect-{addon}"), true).await;
            crate::chrome_control::test_support::answering(&core.shared, |_, _| {
                panic!("facts never call the helper")
            });
            let state = core
                .shared
                .chrome_control
                .config
                .user_home
                .as_ref()
                .unwrap()
                .join("VibeWork/ChromeControl/setup.json");
            std::fs::write(
                state,
                json!({"files":[],"browsers":["Google Chrome","Microsoft Edge"],"selectedBrowsers":["Google Chrome","Microsoft Edge"],"userData":[]})
                    .to_string(),
            )
            .unwrap();
            crate::chrome_control::test_support::observe_status(
                &core.shared,
                &json!({"profiles":[{"profileId":profile,"connected":true}]}),
            );
            crate::chrome_control::test_support::observe_status(
                &core.shared,
                &json!({"profileId":profile,"error":{"code":"OWNER_CONNECTING"}}),
            );
            let status = call(&core.shared, "s", tools::SESSION_STATUS, &json!({}))
                .await
                .unwrap();
            let shown = status["addons"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == addon)
                .unwrap();
            assert_eq!(shown["stateCode"], "connecting");
            assert_eq!(
                shown["state"],
                if addon == "edge" {
                    "Turn on in Edge"
                } else {
                    "Turn on in Chrome"
                }
            );
            let instruction = shown["instruction"].as_str().unwrap();
            assert!(instruction.contains("Extensions page"), "{shown}");
            assert!(
                instruction.contains("turn on Vibe Work if it is off"),
                "{shown}"
            );
            assert!(instruction.contains("restart"), "{shown}");
            assert!(!instruction.contains("Vibe Work icon"), "{shown}");
            let suggested = call(
                &core.shared,
                "s",
                tools::ADDON_SUGGEST,
                &json!({"addon":addon,"reason":"Continue the browser task"}),
            )
            .await;
            assert!(
                suggested.is_err(),
                "a paired reconnect has no owner card: {suggested:?}"
            );
            core.shutdown().await;
        }
    }

    #[cfg(feature = "your-edge")]
    #[tokio::test]
    async fn session_status_and_suggestion_include_edge() {
        let core = chat("session-addon-edge", true).await;
        let status = call(&core.shared, "s", tools::SESSION_STATUS, &json!({}))
            .await
            .unwrap();
        let edge = status["addons"]
            .as_array()
            .unwrap()
            .iter()
            .find(|addon| addon["id"] == "edge");
        assert!(edge.is_some(), "{status}");
        assert_eq!(edge.unwrap()["name"], vw_vocabulary::texts::EDGE_NAME);
        let shown = call(
            &core.shared,
            "s",
            tools::ADDON_SUGGEST,
            &json!({"addon":"edge","reason":"Read your Edge tab"}),
        )
        .await
        .unwrap();
        assert_eq!(shown["addonSuggestion"]["addon"], "edge");
        let instruction = shown["addonSuggestion"]["instruction"].as_str().unwrap();
        assert!(!instruction.contains("opens Settings"));
        assert!(instruction.contains("click Connect Edge."));
        assert!(
            shown["addonSuggestion"]["instruction"]
                .as_str()
                .unwrap()
                .contains("click Connect Edge")
        );
        core.shutdown().await;
    }

    #[cfg(feature = "your-edge")]
    #[tokio::test]
    async fn edge_only_pairing_is_reported_without_offering_to_connect_it_again() {
        let core = chat("session-addon-edge-connected", true).await;
        crate::chrome_control::test_support::answering(&core.shared, |_, _| {
            panic!("facts must not call the helper")
        });
        // A live profile belongs to an installed setup, just as the shared Chrome state requires.
        let state = core
            .shared
            .chrome_control
            .config
            .user_home
            .as_ref()
            .unwrap()
            .join("VibeWork/ChromeControl/setup.json");
        std::fs::write(
            state,
            json!({"files":[],"browsers":["Microsoft Edge"],"userData":[]}).to_string(),
        )
        .unwrap();
        crate::chrome_control::test_support::observe_status(
            &core.shared,
            &json!({"profiles":[{"profileId":"edge:p","connected":true,"extensionVersion":"0.1.3"}]}),
        );
        let status = call(&core.shared, "s", tools::SESSION_STATUS, &json!({}))
            .await
            .unwrap();
        let addons = status["addons"].as_array().unwrap();
        let edge = addons.iter().find(|addon| addon["id"] == "edge").unwrap();
        assert_eq!(edge["name"], vw_vocabulary::texts::EDGE_NAME);
        assert_eq!(edge["stateCode"], "connected", "{status}");
        assert_ne!(addons[0]["stateCode"], "connected");
        let refusal = call(
            &core.shared,
            "s",
            tools::ADDON_SUGGEST,
            &json!({"addon":"edge","reason":"Read your tab"}),
        )
        .await
        .unwrap_err();
        assert!(
            refusal.contains(&format!(
                "{} is already connected",
                vw_vocabulary::texts::EDGE_NAME
            )),
            "{refusal}"
        );
        core.shutdown().await;
    }

    #[tokio::test]
    async fn addon_suggest_shows_the_card_only_when_the_person_has_a_step() {
        let core = chat("session-addon-suggest", true).await;
        let shown = call(
            &core.shared,
            "s",
            tools::ADDON_SUGGEST,
            &json!({"addon":"chrome","reason":"Read the page you have open"}),
        )
        .await
        .unwrap();
        assert_eq!(shown["addonSuggestion"]["addon"], "chrome", "{shown}");
        assert_eq!(
            shown["addonSuggestion"]["state"], "Not connected",
            "{shown}"
        );
        assert_eq!(shown["addonSuggestion"]["stateCode"], "notSetUp", "{shown}");
        assert_eq!(
            shown["addonSuggestion"]["reason"],
            "Read the page you have open"
        );
        for bad in [
            json!({"addon":"chrome"}),
            json!({"addon":"chrome","reason":" "}),
            json!({"addon":"unknown","reason":"x"}),
        ] {
            assert!(
                call(&core.shared, "s", tools::ADDON_SUGGEST, &bad)
                    .await
                    .is_err(),
                "{bad}"
            );
        }
        // Nothing was set up, paired or turned on by the suggestion.
        assert_eq!(
            crate::chrome_control::commands::addon(&core.shared).state,
            crate::chrome_control::AddonState::NotSetUp
        );
        core.shutdown().await;
        let core = chat("session-addon-suggest-none", false).await;
        let refused = call(
            &core.shared,
            "s",
            tools::ADDON_SUGGEST,
            &json!({"addon":"chrome","reason":"x"}),
        )
        .await
        .unwrap_err();
        assert!(refused.contains("ask the user"), "{refused}");
        core.shutdown().await;
    }
}
