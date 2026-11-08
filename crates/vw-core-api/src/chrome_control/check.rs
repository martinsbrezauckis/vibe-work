//! Browser authority from the current chat mode. Called at execution and by tool discovery.
//! No Chrome check opens a card or waits for a decision.
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::Arc;
use vw_ask::chrome::Tier;

pub(crate) fn mode(shared: &Shared, session: &str) -> Option<String> {
    shared.access.remembered(session).or_else(|| {
        shared.store.runtime(session).and_then(|runtime| {
            runtime
                .pointer("/activeAgent/name")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
    })
}
/// A mode reduction revokes the helper lease, including already-installed request rules and emulation.
pub(crate) fn mode_changed(shared: &Shared, session: &str, before: Option<&str>) {
    let old = before.and_then(Tier::from_mode);
    let now = mode(shared, session).as_deref().and_then(Tier::from_mode);
    if old.is_some() && now < old {
        super::end_chat(shared, session, false);
    }
}
/// Whether the session's current mode has a browser tier (tests; the tool list itself is `gateway::list`).
#[cfg(test)]
pub(crate) fn available(shared: &Shared, session: &str) -> bool {
    shared.chrome_control.enabled()
        && mode(shared, session)
            .as_deref()
            .and_then(Tier::from_mode)
            .is_some()
}
pub(super) fn authorize(
    shared: &Shared,
    session: &str,
    tool: &str,
    args: &Value,
) -> Result<Tier, String> {
    let browser = super::agent_browser::selected(shared, session, tool, args);
    authorize_mode(shared, session, tool, args, false).map_err(|reason| browser.text(&reason))
}
pub(super) fn authorize_read(
    shared: &Shared,
    session: &str,
    tool: &str,
    args: &Value,
    consent: Option<&super::read_consent::Consent>,
) -> Result<Tier, String> {
    let browser = super::agent_browser::selected(shared, session, tool, args);
    authorize_mode(
        shared,
        session,
        tool,
        args,
        consent.is_some() && super::read_consent::card_action(tool, args),
    )
    .map_err(|reason| browser.text(&reason))
}
fn authorize_mode(
    shared: &Shared,
    session: &str,
    tool: &str,
    args: &Value,
    read_consent: bool,
) -> Result<Tier, String> {
    let Some(view) = shared.store.view(session) else {
        return Err("Not run: this chat is not open.".into());
    };
    if view.state["session"]["parentSessionId"]
        .as_str()
        .is_some_and(|id| !id.is_empty())
    {
        return Err(subagent_reason(super::agent_browser::selected(
            shared, session, tool, args,
        )));
    }
    if !super::agent_browser::selected(shared, session, tool, args).enabled(shared) {
        let browser = super::agent_browser::selected(shared, session, tool, args);
        return Err(super::availability::reason(&browser.addon(shared), browser));
    }
    let mode = mode(shared, session)
        .ok_or_else(|| "Not run: this chat's access mode is unavailable.".to_owned())?;
    let tier =
        Tier::from_mode(&mode).ok_or("Not run: Chrome is unavailable in this access mode.")?;
    let required = super::mapping::required(tool, args).ok_or_else(|| {
        match super::mapping::action(tool, args).as_str() {
            "client.selection" => "Not run: selection is sent by the person from Chrome. Use Ask Vibe Work in Chrome's context menu.",
            "page.dragdrop" => "Not run: page.dragdrop is a permission name. Use page.drag; ask chrome_status for its schema.",
            _ => "Not run: unknown Chrome action. Use chrome_status to find supported actions.",
        }
    })?;
    if required > tier && !(tier == Tier::Ask && read_consent) {
        let label = if mode == "smart-approve" {
            vw_vocabulary::label("accessModes", "smart-approve")
        } else {
            tier.label()
        };
        let action = super::mapping::action(tool, args);
        let action = if action == "page.commit" {
            "sending this form"
        } else {
            action.as_str()
        };
        return Err(format!(
            "Not run: {action} needs {}; this chat is in {label}. Use an allowed action or ask the person to change the chat mode.",
            required.required_label()
        ));
    }
    Ok(tier)
}

fn subagent_reason(browser: super::agent_browser::Browser) -> String {
    use vw_agent_tools::wire::tools;
    format!(
        "Not run: subagents cannot use {}. Use Vibe Work's built-in browser ({} / {}) instead, or ask the main chat for steps in {}.",
        browser.name(),
        tools::BROWSER_READ,
        tools::BROWSER_ACT,
        browser.name()
    )
}
/// `chrome_status`'s action list: each action says whether this chat's current mode allows it, and if not, the modes
/// that would. Read at each call, so a mode change shows at once; the call-time [`authorize`] stays the authority.
pub(super) fn annotate_modes(shared: &Shared, session: &str, tool: &str, result: &mut Value) {
    if tool != "chrome_status" {
        return;
    }
    let selected_profile = shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(session)
        .cloned();
    super::capabilities::filter(result, selected_profile.as_deref());
    let tier = mode(shared, session).as_deref().and_then(Tier::from_mode);
    // Older helpers can return delivery events or permission names among executable features.
    let known = |action: &Value| {
        super::mapping::required("chrome_execute", &json!({"action":action})).is_some()
    };
    if let Some(profiles) = result["structuredContent"]["profiles"].as_array_mut() {
        // The helper retains disconnected identities for silent reconnect. They are not current choices;
        // a deleted Chrome profile cannot reconnect, and its old ID must not confuse the agent's list.
        // This is a projection only: never erase a pairing just because a browser is not running.
        profiles.retain(|profile| profile["connected"] != false);
        for profile in profiles {
            if let Some(features) = profile["features"].as_array_mut() {
                features.retain(known);
            }
        }
    }

    // Always-on legacy address policy, and whether the running helper acknowledged it.
    if result["structuredContent"].is_object() {
        if shared.chrome_control.config.available() {
            let browser = super::agent_browser::selected(shared, session, tool, &json!({}));
            let addon = browser.addon(shared);
            result["structuredContent"]["addon"] = json!({
                "state":addon.label, "stateCode":addon.state, "instruction":addon.instruction,
                "extensionVersion":addon.extension_version,
            });
        }
        result["structuredContent"]["chromeExtensionStore"] =
            json!(super::commands::extension_store(shared));
        result["structuredContent"]["localNetwork"] = json!({
            "on": shared.chrome_control.local_network(),
            "applied": shared.chrome_control.local_network_applied(),
        });
    }
    let Some(rows) = result["structuredContent"]["capabilities"]["actions"].as_array_mut() else {
        return;
    };
    rows.retain(|row| known(&row["action"]));
    for row in rows {
        let action = row["action"].as_str().unwrap_or_default().to_owned();
        // page.key's tier depends on the key: navigation keys (Tab, Escape, arrows…) are marked; typing keys are noted.
        let args = if action == "page.key" {
            json!({"key":"Tab"})
        } else {
            json!({})
        };
        let Some(needed) =
            super::mapping::required("chrome_execute", &json!({"action":action,"args":args}))
        else {
            continue;
        };
        let allowed = tier.is_some_and(|tier| needed <= tier);
        row["allowedInThisMode"] = json!(allowed);
        let consent = tier == Some(Tier::Ask)
            && matches!(
                action.as_str(),
                "history.read" | "bookmarks.read" | "tabs.close_owned" | "tabs.close_user"
            );
        if consent {
            row["approvalRequired"] = json!(true);
        } else if !allowed {
            row["needs"] = json!(needed.required_label());
        }
        if action == "page.key" && tier.is_none_or(|tier| tier < Tier::AcceptEdits) {
            row["typingKeysNeed"] = json!(Tier::AcceptEdits.required_label());
        }
    }
}
pub(crate) fn check(shared: &Arc<Shared>, args: &Value) -> Value {
    let source = args["sessionId"].as_str().unwrap_or_default();
    let session = args["sessionId"].as_str().unwrap_or_default();
    let session = if shared.store.view(session).is_some() {
        session
    } else {
        args["parentSessionId"].as_str().unwrap_or(session)
    };
    let tool = args["toolName"]
        .as_str()
        .unwrap_or_default()
        .strip_prefix("mcp_vw.")
        .unwrap_or(args["toolName"].as_str().unwrap_or_default());
    // Without the package every Chrome tool is refused here too, with the same sentence as the tool itself.
    if !shared.chrome_control.config.available() {
        let browser = super::agent_browser::selected(shared, session, tool, &args["toolInput"]);
        return json!({"decision":"deny","reason":browser.text(super::setup::ABSENT)});
    }
    let child = args["parentSessionId"]
        .as_str()
        .is_some_and(|parent| !parent.is_empty() && parent != source)
        || shared.store.view(source).is_some_and(|view| {
            view.state["session"]["parentSessionId"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        });
    if child {
        let browser = super::agent_browser::selected(shared, session, tool, &args["toolInput"]);
        return json!({"decision":"deny","reason":subagent_reason(browser)});
    }
    // Execute performs the tier check itself so the refusal reaches the model as a tool receipt.
    if tool == "chrome_execute" {
        return json!({"decision":"allow"});
    }
    match authorize(shared, session, tool, &args["toolInput"]) {
        Ok(_) => json!({"decision":"allow"}),
        Err(reason) => json!({"decision":"deny","reason":reason}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn chrome_status_does_not_invent_an_unknown_store_state() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let mut reply = json!({"structuredContent":{"profiles":[]}});
        annotate_modes(&fixture.core.shared, "s", "chrome_status", &mut reply);
        fixture.finish().await;
        assert_eq!(
            reply["structuredContent"].get("chromeExtensionStore"),
            Some(&Value::Null)
        );
    }
    #[tokio::test]
    async fn current_mode_controls_every_call_without_cards() {
        let core = crate::offline_core::start("chrome-current-mode").await;
        let shared = &core.shared;
        let _ = shared
            .store
            .load(
                json!({"session":{"id":"s"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        for (mode, allowed) in [
            ("ask", false),
            ("accept-edits", true),
            ("smart-approve", true),
            ("auto-approve", true),
            ("ask", false),
        ] {
            shared
                .access
                .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
            let typed = authorize(
                shared,
                "s",
                "chrome_execute",
                &json!({"action":"page.type"}),
            );
            assert_eq!(typed.is_ok(), allowed, "{mode}: {typed:?}");
            if !allowed {
                assert!(
                    typed
                        .unwrap_err()
                        .contains(&Tier::AcceptEdits.required_label())
                );
            }
            assert_eq!(shared.asks.list(), [] as [vw_ask::Ask; 0]);
        }
        for mode in ["vw-plan", "explore", "unknown"] {
            shared
                .access
                .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
            assert!(!available(shared, "s"));
            // The tool list does not follow the chat (`gateway` tests); the call is refused.
            assert!(authorize(shared, "s", "chrome_status", &json!({})).is_err());
        }
        core.shutdown().await;
    }

    /// Seen in a survey run: in Ask, `chrome_status` showed typing as available; the refusal came only at the
    /// call. PREDICT (before the change): the listed actions carry no fact about this chat's current mode.
    #[tokio::test]
    async fn the_action_list_says_what_this_chats_current_mode_allows() {
        let core = crate::offline_core::start_with("chrome-status-modes", |config, dir| {
            config.chrome_control.package = Some(dir.join("chrome-package"));
        })
        .await;
        let shared = &core.shared;
        let _ = shared
            .store
            .load(
                json!({"session":{"id":"s"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        let _seen = super::super::test_support::answering(shared, |_, _| {
            json!({"isError":false,"content":[{"type":"text","text":"status"}],"structuredContent":{"capabilities":{
                "actions":[{"action":"page.read"},{"action":"page.type"},{"action":"raw.execute"},{"action":"page.key"}]}}})
        });
        for (mode, typing, script) in [
            ("ask", false, false),
            ("accept-edits", true, false),
            ("auto-approve", true, true),
        ] {
            shared
                .access
                .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
            let status = super::super::gateway::call(shared, "s", "chrome_status", json!({}))
                .await
                .unwrap();
            let rows =
                status["vwMcpResult"]["structuredContent"]["untrustedPageContent"]["capabilities"]
                    ["actions"]
                    .as_array()
                    .unwrap()
                    .clone();
            let row = |id: &str| rows.iter().find(|r| r["action"] == id).unwrap().clone();
            assert_eq!(row("page.read")["allowedInThisMode"], true, "{mode}");
            assert_eq!(row("page.type")["allowedInThisMode"], typing, "{mode}");
            assert_eq!(row("raw.execute")["allowedInThisMode"], script, "{mode}");
            if !typing {
                assert_eq!(
                    row("page.type")["needs"],
                    Tier::AcceptEdits.required_label()
                );
            }
            if !script {
                assert_eq!(
                    row("raw.execute")["needs"],
                    Tier::FullAccess.required_label()
                );
            }
            assert!(row("page.read").get("needs").is_none());
            // W3 T11: page.key was marked not allowed in Ask, yet Tab and Escape ran there; the mark follows the
            // navigation keys, and the keys that type say what they need.
            assert_eq!(row("page.key")["allowedInThisMode"], true, "{mode}");
            assert_eq!(
                row("page.key").get("typingKeysNeed").cloned(),
                (!typing).then(|| json!(Tier::AcceptEdits.required_label())),
                "{mode}"
            );
            // The model reads the text part: the same facts are there.
            let text = status["vwMcpResult"]["content"][0]["text"]
                .as_str()
                .unwrap();
            assert!(text.contains("allowedInThisMode"), "{text}");
        }
        core.shutdown().await;
    }
}

#[cfg(test)]
mod unsupported_tests {
    use super::*;

    #[tokio::test]
    async fn older_helpers_cannot_advertise_unmapped_actions() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let mut reply = json!({"structuredContent":{
            "profiles":[{"features":["client.selection","page.dragdrop","page.drag","page.read"]}],
            "capabilities":{"actions":[{"action":"client.selection"},{"action":"page.dragdrop"},{"action":"page.drag"}]}
        }});
        annotate_modes(&fixture.core.shared, "s", "chrome_status", &mut reply);
        assert_eq!(
            reply["structuredContent"]["profiles"][0]["features"],
            json!(["page.drag", "page.read"])
        );
        assert_eq!(
            reply["structuredContent"]["capabilities"]["actions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        fixture.finish().await;
    }

    #[tokio::test]
    async fn delivery_events_and_permission_names_explain_the_supported_route() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let shared = &fixture.core.shared;
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
        for (action, reason, instruction) in [
            ("client.selection", "person", "Ask Vibe Work"),
            ("page.dragdrop", "permission", "page.drag"),
        ] {
            let error =
                authorize(shared, "s", "chrome_execute", &json!({"action":action})).unwrap_err();
            assert!(
                error.contains(reason) && error.contains(instruction),
                "{error}"
            );
        }
        fixture.finish().await;
    }
}

#[cfg(test)]
#[path = "check_subagent_tests.rs"]
mod subagent_tests;
