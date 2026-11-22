//! Compiles named page operations into fixed extension primitives after the shared mode/ownership checks.
//! Only this module adds raw transport authority; callers cannot supply steps, code or capabilities.
use super::{owner, read_consent::Consent};
use crate::core::Shared;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub(super) fn handles(action: &str) -> bool {
    super::page_plan_catalog::NAMES.contains(&action)
        || super::extension_state::NAMES.contains(&action)
        || super::page_capture::NAMES.contains(&action)
}

pub(super) fn supported(profile: &Value) -> bool {
    profile["connected"] == true
        && profile["extensionCapabilities"]
            .as_array()
            .is_some_and(|caps| {
                caps.iter().any(|c| {
                    c == "transport.contract.v1" || c == "firefox.page.v1" || c == "safari.page.v1"
                })
            })
}

pub(super) fn supported_action(profile: &Value, action: &str) -> bool {
    if profile["extensionCapabilities"]
        .as_array()
        .is_some_and(|caps| caps.contains(&json!("safari.page.v1")))
    {
        return supported(profile)
            && profile["features"]
                .as_array()
                .is_some_and(|names| names.contains(&json!(action)));
    }
    supported(profile)
        && (!profile["extensionCapabilities"]
            .as_array()
            .is_some_and(|caps| caps.contains(&json!("firefox.page.v1")))
            || action.starts_with("media.")
            || action.starts_with("recording.video.")
            || super::page_capture::NAMES.contains(&action))
}

fn primitive_failure(error: owner::RpcFailure, action: &str) -> String {
    match error {
        owner::RpcFailure::Revoked => crate::ask::tool_calls::REVOKED_BROWSER_GRANT.into(),
        owner::RpcFailure::Unavailable => "The page operation could not be acknowledged; its outcome is unknown. Check browser state before trying again.".into(),
        owner::RpcFailure::Rejected { code, helper_code, message } => {
            // These are fixed helper admissions, not page-derived strings. A future helper's
            // arbitrary error text may contain page data, so only known advice is shown verbatim.
            const UNKNOWN_HISTORY: &str = "Not run: this document's page-script or vault-fill history could not be verified after reconnecting. Reload or navigate the tab to a new document before filling or running JavaScript.";
            const STALE_TAB: &str =
                "Not run: no tab with this ID is available. Use a tabId from tabs.list";
            const TIMEOUT_HELPER: &str = "The browser has not yet confirmed that the timed-out action stopped; the page may still be busy. Wait a few seconds, then try again in this browser session.";
            const ADVICE: &[&str] = &[
                UNKNOWN_HISTORY,
                STALE_TAB,
                TIMEOUT_HELPER,
                "Session start must be acknowledged",
                "Settle the current operation before another admission",
                "Another browser chat still has work on this tab. Use a tab in this chat's window, or wait for that work to finish.",
            ];
            let known_helper_code = match message.as_str() {
                UNKNOWN_HISTORY => Some("VAULT_SCRIPT_CONFLICT"),
                STALE_TAB => Some("UNSUPPORTED_TARGET"),
                TIMEOUT_HELPER | "Session start must be acknowledged" => Some("CONTROL_SUSPENDED"),
                "Settle the current operation before another admission" => Some("OPERATION_BUSY"),
                "Another browser chat still has work on this tab. Use a tab in this chat's window, or wait for that work to finish." => Some("TAB_BUSY"),
                _ => None,
            };
            let rejection = if let Some(known) = known_helper_code.filter(|known| helper_code.as_deref() == Some(*known)) {
                format!("browser rejection {code} ({known})")
            } else {
                format!("browser rejection {code}")
            };
            if code == -32602
                && message == UNKNOWN_HISTORY
                && matches!(action, "capture.page" | "capture.media")
            {
                format!("Capture was not run because Vibe Work cannot verify this page's earlier use of secrets. For your own agent tab, reload or navigate it, then capture again. For the user's tab, ask the user to reload or navigate it. ({rejection})")
            } else if code == -32602
                && ADVICE.contains(&message.as_str())
                && (message != STALE_TAB || helper_code.as_deref() == Some("UNSUPPORTED_TARGET"))
            {
                let advice = if message == TIMEOUT_HELPER {
                    message.replace("in this session.", "in this browser session.")
                } else {
                    message
                };
                format!("{advice} ({rejection})")
            } else {
                format!("The browser controller refused this page operation (code {code}); check browser state before trying again.")
            }
        }
    }
}

#[cfg(test)]
mod primitive_failure_tests {
    use super::{owner::RpcFailure, primitive_failure};

    /// PREDICT: a closed tab after reconnect is reported by the helper as `UNSUPPORTED_TARGET`,
    /// but capture loses the useful `tabs.list` continuation while `page.read` preserves it.
    #[test]
    fn stale_capture_tab_gives_the_vetted_tab_list_continuation() {
        const MESSAGE: &str =
            "Not run: no tab with this ID is available. Use a tabId from tabs.list";
        for action in ["capture.page", "capture.media"] {
            let text = primitive_failure(
                RpcFailure::Rejected {
                    code: -32602,
                    helper_code: Some("UNSUPPORTED_TARGET".into()),
                    message: MESSAGE.into(),
                },
                action,
            );
            assert!(text.contains(MESSAGE), "{action}: {text}");
            assert!(text.contains("UNSUPPORTED_TARGET"), "{action}: {text}");
        }
        for (code, message) in [
            ("OTHER", MESSAGE),
            (
                "UNSUPPORTED_TARGET",
                "Not run: no tab with this ID is available. SECRET_SENTINEL",
            ),
        ] {
            let text = primitive_failure(
                RpcFailure::Rejected {
                    code: -32602,
                    helper_code: Some(code.into()),
                    message: message.into(),
                },
                "capture.page",
            );
            assert!(!text.contains("tabs.list"), "{text}");
            assert!(!text.contains("SECRET_SENTINEL"), "{text}");
        }
    }

    #[test]
    fn transport_loss_does_not_claim_a_refusal_or_a_safe_retry() {
        let text = primitive_failure(RpcFailure::Unavailable, "capture.page");
        assert!(text.contains("outcome is unknown"), "{text}");
        assert!(!text.contains("refused"), "{text}");
    }

    #[test]
    fn known_helper_timeout_keeps_matching_original_advice_and_names_browser_session() {
        let text = primitive_failure(
            RpcFailure::Rejected {
                code: -32602,
                helper_code: Some("CONTROL_SUSPENDED".into()),
                message: "The browser has not yet confirmed that the timed-out action stopped; the page may still be busy. Wait a few seconds, then try again in this browser session.".into(),
            },
            "page.read",
        );
        assert!(
            text.contains("try again in this browser session."),
            "{text}"
        );
        assert!(text.contains("CONTROL_SUSPENDED"), "{text}");
    }

    #[test]
    fn unexpected_helper_text_cannot_leak_page_or_secret_data() {
        let text = primitive_failure(
            RpcFailure::Rejected {
                code: -32602,
                helper_code: Some("SECRET_CODE_SENTINEL".into()),
                message: "unexpected page text SECRET_SENTINEL".into(),
            },
            "capture.page",
        );
        assert!(text.contains("code -32602"), "{text}");
        assert!(!text.contains("SECRET_SENTINEL"), "{text}");
        assert!(!text.contains("SECRET_CODE_SENTINEL"), "{text}");
    }
}

fn checked(
    shared: &Shared,
    chat: &str,
    request: &Value,
    consent: Option<&Consent>,
) -> Result<(), String> {
    super::check::authorize_read(shared, chat, "chrome_execute", request, consent)?;
    super::gateway::validate_session(shared, chat, "chrome_execute", request)?;
    super::reconnect::stopped(
        shared,
        chat,
        super::agent_browser::selected(shared, chat, "chrome_execute", request),
    )
}

pub(super) async fn dispatch(
    shared: &Arc<Shared>,
    chat: &str,
    request: &Value,
    consent: Option<&Consent>,
    steps: Value,
) -> Result<Value, String> {
    checked(shared, chat, request, consent)?;
    let tab = request["args"]["tabId"].as_u64().filter(|id| i32::try_from(*id).is_ok()).ok_or("Not run: this action needs an exact tabId from this chat's tabs.list; select its page first.")?;
    let helper_session = request["sessionId"]
        .as_str()
        .ok_or("A browser session is required")?;
    let operation = shared
        .chrome_control
        .external_video
        .operation(helper_session);
    let _operation_guard = operation.lock().await;
    let status = shared
        .chrome_control
        .owner
        .call("chrome_status", json!({}))
        .await;
    let body = &status.value()["structuredContent"];
    let profile = body["sessions"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["sessionId"] == request["sessionId"]))
        .and_then(|r| r["profileId"].as_str());
    if !body["profiles"].as_array().is_some_and(|rows| {
        rows.iter().any(|r| {
            profile.is_some_and(|id| r["profileId"] == id)
                && supported_action(r, request["action"].as_str().unwrap_or_default())
        })
    }) {
        return Err("Not run: this browser connection does not support the requested page action. Use this browser's listed actions, or enable its current browser connection.".into());
    }
    checked(shared, chat, request, consent)?;
    // The fixed permission probe needs page.read in addition to raw transport.
    // Other plans keep their narrower grant; callers cannot supply these steps.
    let mut capabilities = vec!["raw.execute"];
    if steps.as_array().is_some_and(|steps| {
        steps
            .iter()
            .any(|step| step["op"] == "api" && step["method"] == "permissions.status")
    }) {
        capabilities.push("page.read");
    }
    super::batch::admit()?;
    let mut result = shared.chrome_control.owner.rpc_result("owner.primitive", json!({"sessionId":request["sessionId"],"tabId":tab,"steps":steps,"approvedCapabilities":capabilities})).await.map_err(|error| primitive_failure(error, request["action"].as_str().unwrap_or_default()))?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    while matches!(result["status"].as_str(), Some("queued" | "running")) {
        if tokio::time::Instant::now() >= deadline {
            return Err("The page operation did not finish within 25 seconds; its outcome is unknown. Read browser state before deciding whether to try again.".into());
        }
        let operation = result["operationId"].as_str().ok_or("The page operation has no receipt identity; inspect browser state before trying again.")?.to_owned();
        tokio::time::sleep(Duration::from_millis(100)).await;
        checked(shared, chat, request, consent)?;
        result = shared
            .chrome_control
            .owner
            .rpc_result(
                "owner.primitiveResult",
                json!({"sessionId":request["sessionId"],"operationId":operation}),
            )
            .await
            .map_err(|error| {
                primitive_failure(error, request["action"].as_str().unwrap_or_default())
            })?;
    }
    checked(shared, chat, request, consent)?;
    result["action"] = request["action"].clone();
    result["capability"] = json!(super::mapping::capability("chrome_execute", request));
    Ok(result)
}

/// Whether the browser profile of this request's session reports page video (`page.video.v1` and the action).
async fn records_video(shared: &Arc<Shared>, request: &Value) -> bool {
    let status = shared
        .chrome_control
        .owner
        .call("chrome_status", json!({}))
        .await;
    let body = &status.value()["structuredContent"];
    let Some(profile) = body["sessions"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["sessionId"] == request["sessionId"]))
        .and_then(|r| r["profileId"].as_str())
    else {
        return false;
    };
    body["profiles"].as_array().is_some_and(|rows| {
        rows.iter().any(|r| {
            r["profileId"] == profile
                && r["connected"] == true
                && r["extensionCapabilities"]
                    .as_array()
                    .is_some_and(|caps| caps.contains(&json!("page.video.v1")))
                && r["features"]
                    .as_array()
                    .is_some_and(|names| names.contains(&request["action"]))
        })
    })
}

pub(super) fn mcp(body: &Value) -> owner::Answer {
    owner::Answer::Helper(
        json!({"isError":body["status"] != "completed","content":[{"type":"text","text":body.to_string()}],"structuredContent":body}),
    )
}

pub(super) async fn send(
    shared: &Arc<Shared>,
    chat: &str,
    request: &Value,
    consent: Option<&Consent>,
) -> Result<owner::Answer, String> {
    if request["action"]
        .as_str()
        .is_some_and(|name| name.starts_with("recording.video."))
    {
        checked(shared, chat, request, consent)?;
        // Extensions that report page.video.v1 record the chat's own tab themselves; the helper admits the action.
        let status = shared
            .chrome_control
            .owner
            .call("chrome_status", json!({}))
            .await;
        if super::external_video::supports(status.value(), request) {
            return super::external_video::send(shared, chat, request, consent).await;
        }
        if records_video(shared, request).await {
            return super::escalation::send(
                shared,
                chat,
                "chrome_execute",
                request.clone(),
                consent,
            )
            .await;
        }
        return Err("Continuous page video is unavailable in this browser connection: its extension has no video recorder (the Chrome/Edge extension records video from version 0.1.12). Use recording.start and recording.stop for a GIF of action screenshots, or ask the user to update the browser extension.".into());
    }
    if super::agent_browser::selected(shared, chat, "chrome_execute", request)
        == super::agent_browser::Browser::Safari
        && request["action"].as_str().is_some_and(|name| {
            name.starts_with("media.") || matches!(name, "tabs.audio" | "tabs.mute")
        })
    {
        checked(shared, chat, request, consent)?;
        let answer =
            super::escalation::send(shared, chat, "chrome_execute", request.clone(), consent)
                .await?;
        checked(shared, chat, request, consent)?;
        return Ok(answer);
    }
    if request["action"]
        .as_str()
        .is_some_and(|a| super::page_capture::NAMES.contains(&a))
    {
        return super::page_capture::send(shared, chat, request, consent).await;
    }
    if request["action"]
        .as_str()
        .is_some_and(|a| super::extension_state::NAMES.contains(&a))
    {
        return super::extension_state::send(shared, chat, request, consent).await;
    }
    if matches!(request["action"].as_str(), Some("tabs.audio" | "tabs.mute")) {
        return tab_audio(shared, chat, request, consent).await;
    }
    let mut args = request["args"].clone();
    if args.as_object().is_none_or(|a| {
        a.keys()
            .any(|k| !["tabId", "mediaId", "timeMs", "muted"].contains(&k.as_str()))
    }) {
        return Err(
            "Not run: unknown media argument; use chrome_status for this action's schema.".into(),
        );
    }
    args["action"] = request["action"].clone();
    let media = vw_browser::playback::Request::parse(&args)?;
    let script = media.script(None);
    // The fixed script owns the promise; the primitive waits for its readback in the same isolated world.
    let poll = media.script(Some("__PLAY_TOKEN__"));
    let expression = format!(
        "(async()=>{{let result=JSON.parse({script});const deadline=Date.now()+5000;while(result.pending&&Date.now()<deadline){{const token=result.token;await new Promise(resolve=>setTimeout(resolve,50));const next=JSON.parse((()=>{{const __token=token;return ({poll});}})());result={{...next,token}};}}if(result.pending)return JSON.stringify({{error:'Media operation did not settle; read media state before trying again.'}});return JSON.stringify(result);}})()",
        poll = poll.replace("\"__PLAY_TOKEN__\"", "__token")
    );
    let mut result = dispatch(shared, chat, request, consent, json!([{"id":"media","op":"cdp","method":"Runtime.evaluate","world":"agent","params":{"expression":expression,"returnByValue":true,"awaitPromise":true}}])).await?;
    if result["status"] == "completed" {
        let data = result.pointer("/data/results/media/result/value").and_then(Value::as_str).and_then(|s| serde_json::from_str::<Value>(s).ok()).ok_or("Media state was not readable in the receipt; inspect the page before trying again.")?;
        if data.get("error").is_some() {
            result["status"] = json!("failed");
        }
        result["data"] = data;
    }
    Ok(mcp(&result))
}

async fn tab_audio(
    shared: &Arc<Shared>,
    chat: &str,
    request: &Value,
    consent: Option<&Consent>,
) -> Result<owner::Answer, String> {
    let input = &request["args"];
    if input
        .as_object()
        .is_none_or(|a| a.keys().any(|k| !["tabId", "muted"].contains(&k.as_str())))
    {
        return Err(
            "Not run: tab audio accepts only tabId and muted; use its advertised schema.".into(),
        );
    }
    let mut steps = Vec::new();
    if request["action"] == "tabs.mute" {
        let muted = input["muted"]
            .as_bool()
            .ok_or("Tab mute needs muted: true or false.")?;
        steps.push(
            json!({"id":"change","op":"api","method":"tabs.update","params":{"muted":muted}}),
        );
    }
    steps.push(json!({"id":"state","op":"api","method":"tabs.get"}));
    let mut result = dispatch(shared, chat, request, consent, json!(steps)).await?;
    if result["status"] == "completed" {
        let state = &result["data"]["results"]["state"];
        if !state["muted"].is_boolean()
            || request["action"] == "tabs.mute" && state["muted"] != input["muted"]
        {
            return Err(
                "Native tab mute was not confirmed; read tabs.audio before trying again.".into(),
            );
        }
        result["data"] =
            json!({"tabId":state["tabId"],"muted":state["muted"],"audible":state["audible"]});
    }
    Ok(mcp(&result))
}

#[cfg(test)]
mod safari_tests {
    use super::*;
    #[test]
    fn safari_named_transport_advertises_only_implemented_semantics() {
        let profile = json!({"connected":true,"extensionCapabilities":["safari.page.v1"],"features":["media.read","media.mute","tabs.audio","tabs.mute"]});
        for action in ["media.read", "media.mute", "tabs.audio", "tabs.mute"] {
            assert!(supported_action(&profile, action), "{action}");
            assert_eq!(
                super::super::mapping::capability("chrome_execute", &json!({"action":action})),
                Some("page.read")
            );
        }
        for action in [
            "capture.page",
            "media.seek",
            "extensions.list",
            "raw.execute",
        ] {
            assert!(!supported_action(&profile, action), "{action}");
        }
        let mut disconnected = profile;
        disconnected["connected"] = json!(false);
        assert!(!supported(&disconnected));
    }
}
