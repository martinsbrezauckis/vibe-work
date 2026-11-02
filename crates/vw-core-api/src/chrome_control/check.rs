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
    if shared.store.view(session).is_none() {
        return Err("Not run: this chat is not open.".into());
    }
    if !shared.chrome_control.enabled() {
        return Err(format!(
            "Not run: {} is off.",
            vw_vocabulary::label("builtinSkills", "vw:chrome")
        ));
    }
    let mode = mode(shared, session).ok_or("Not run: this chat's access mode is unavailable.")?;
    let tier =
        Tier::from_mode(&mode).ok_or("Not run: Chrome is unavailable in this access mode.")?;
    let required = super::mapping::required(tool, args).ok_or("Not run: unknown Chrome action.")?;
    if required > tier {
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
            "Not run: {action} needs {}; this chat is in {label}.",
            required.required_label()
        ));
    }
    Ok(tier)
}
/// `chrome_status`'s action list: each action says whether this chat's current mode allows it, and if not, the modes
/// that would. Read at each call, so a mode change shows at once; the call-time [`authorize`] stays the authority.
pub(super) fn annotate_modes(shared: &Shared, session: &str, tool: &str, result: &mut Value) {
    if tool != "chrome_status" {
        return;
    }
    let tier = mode(shared, session).as_deref().and_then(Tier::from_mode);
    // The person's Local network switch, and whether Chrome has it yet.
    if result["structuredContent"].is_object() {
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
        if !allowed {
            row["needs"] = json!(needed.required_label());
        }
        if action == "page.key" && tier.is_none_or(|tier| tier < Tier::AcceptEdits) {
            row["typingKeysNeed"] = json!(Tier::AcceptEdits.required_label());
        }
    }
}
pub(crate) fn check(shared: &Arc<Shared>, args: &Value) -> Value {
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
        return json!({"decision":"deny","reason":super::setup::ABSENT});
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
        let _seen = shared.chrome_control.owner.answering(|_, _| {
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
