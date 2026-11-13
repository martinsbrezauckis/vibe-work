//! Session-authenticated Chrome tools. Each call uses the current chat mode.
use super::{
    agent_browser::{self, Browser},
    owner,
};
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::{Arc, PoisonError};

pub(crate) fn is_tool(name: &str) -> bool {
    owner::TOOLS.contains(&name)
}

/// Vibe caches tools per server launch line for up to a day. Package presence controls this shared list;
/// `agent_tools::chrome_launch` chooses the chat's tools, and `check::authorize` rechecks its mode and switch.
pub(crate) fn list(shared: &Shared) -> Value {
    // The core answers this list directly, before `tools::call`'s package check.
    if !shared.chrome_control.config.available() {
        return json!([]);
    }
    json!(vw_agent_tools::chrome_tool_list())
}

pub(super) fn validate_public_args(args: &Value) -> Result<(), String> {
    if args.as_object().is_none_or(|args| {
        args.keys().any(|key| {
            key.starts_with('_')
                || key == "approvedCapabilities"
                || key == "clientSessionId"
                || key == "sessionFolder"
                || key == "loginValue"
                || key == "preflightOperationId"
        })
    }) {
        return Err("Not run: invalid Chrome tool arguments.".to_owned());
    }
    Ok(())
}

/// Keeps the single-action/batch contract explicit while native discovery receives a flat object schema.
pub(crate) fn validate_execute_args(args: &Value) -> Result<(), String> {
    validate_public_args(args)?;
    super::output_file::validate(args)?;
    if args.get("steps").is_some() {
        return super::batch::steps(args).map(|_| ());
    }
    if !args["action"].is_string() || !args["args"].is_object() {
        return Err("Not run: an action takes action and args, or use a batch of steps.".into());
    }
    Ok(())
}

pub(super) fn validate_session_args(args: &Value) -> Result<(), String> {
    validate_public_args(args)?;
    super::profile_ids::validate_browser(args)?;
    if args.get("title").is_some() {
        return Err("Not run: Vibe Work supplies this chat's browser session title; omit title from the tool arguments.".into());
    }
    let field = match args["action"].as_str() {
        Some("open") => "profileId",
        Some("get" | "cancel" | "stop") => "sessionId",
        _ => {
            return Err(
                "Not run: chrome_session requires action: open, get, cancel or stop.".into(),
            );
        }
    };
    if args[field].as_str().is_none_or(|id| id.trim().is_empty()) {
        return Err(format!(
            "Not run: chrome_session requires {field} for this action."
        ));
    }
    Ok(())
}

pub(super) fn validate_session(
    shared: &Shared,
    session: &str,
    tool: &str,
    args: &Value,
) -> Result<(), String> {
    if let Some(task) = args["sessionId"].as_str() {
        let owner = shared
            .chrome_control
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(task)
            .cloned();
        if owner.as_deref() != Some(session) {
            // Ask-mode consent, login and batch preparation return this refusal without `call`'s browser
            // mapping, so name the session's own browser here (`text` is idempotent on already-named text).
            let browser = agent_browser::selected(shared, session, tool, args);
            return Err(browser.text(refusal(shared, session, task, owner.as_deref())));
        }
    } else if tool == "chrome_execute" {
        let browser = agent_browser::selected(shared, session, tool, args);
        return Err(browser.text("Not run: open a Chrome session in this chat first."));
    }
    Ok(())
}

/// The refusal of another chat's Chrome session. It is a fact about ownership, never about the connection, so it
/// stays the answer while nothing is connected too ([`lease_refusal`]).
const OTHER: &str = "Not run: this Chrome session belongs to another chat.";

/// A lease or support refusal while the card says nothing usable is connected: the card's diagnosis and the person's
/// step replace it (an old session id after Chrome disconnected says why Chrome is unavailable, not "unknown id").
/// Another chat's session keeps its own refusal.
fn lease_refusal(
    shared: &Shared,
    browser: Browser,
    live: bool,
    reason: String,
) -> Result<Value, String> {
    // `validate_session` names the session's own browser, so match the ownership refusal in every browser's words.
    if [
        Browser::Chrome,
        Browser::Edge,
        Browser::Safari,
        Browser::Firefox,
    ]
    .into_iter()
    .any(|named| reason == named.text(OTHER))
    {
        return Err(reason);
    }
    super::availability::refusal(shared, browser, live).ok_or(reason)
}

/// Why `task` is not this chat's live session, in the words the agent reads: this chat's own session that is not
/// retained after a turn says what ended it; a known session of another chat says so; any other id is unknown.
fn refusal(shared: &Shared, session: &str, task: &str, owner: Option<&str>) -> &'static str {
    if owner.is_some() {
        return OTHER;
    }
    let chrome = &shared.chrome_control;
    let shown = {
        let work = chrome.work.lock().unwrap_or_else(PoisonError::into_inner);
        work.chat_of(task).map(|chat| {
            (
                chat == session,
                work.state_of(task, chat).unwrap_or_default().to_owned(),
            )
        })
    };
    match shown {
        Some((true, state)) if state == "disconnected" => {
            "Not run: this Chrome session lost its connection to Chrome."
        }
        Some((true, _)) => {
            "Not run: this Chrome session was stopped; open a new one with chrome_session."
        }
        Some((false, _)) => OTHER,
        // After an app restart an opaque old ID has no browser binding. The browser that
        // happens to be connected or selected in this chat cannot name that missing session.
        None => "Not run: no browser session with this id; open one with chrome_session.",
    }
}

pub(crate) async fn call(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    let browser = agent_browser::selected(shared, session, tool, &args);
    if tool == "chrome_execute" {
        super::reconnect::stopped(shared, session, browser)?;
        shared.browser_dialogs.check()?;
    }
    if matches!(tool, "chrome_status" | "chrome_execute")
        && let Some(action) = args["action"].as_str()
        && let Some(answer) = super::output_status::unknown_action(action)
    {
        validate_public_args(&args)?;
        let mut answer = answer;
        super::receipt::stamp(shared, tool, &args, &mut answer);
        super::output::guard(&mut answer);
        return Ok(json!({"vwMcpResult":answer}));
    }
    if tool == "chrome_status" {
        validate_public_args(&args)?;
        if let Some(actions) = super::status_actions::requested(&args)? {
            return super::status_actions::call(
                shared,
                session,
                &actions,
                args["profileId"].as_str(),
            )
            .await
            .map(|mut answer| {
                super::profile_agent::status(shared, session, &mut answer["vwMcpResult"]);
                super::output::guard_status(&mut answer["vwMcpResult"]);
                answer
            })
            .inspect(|answer| super::connection_notice::refused(shared, session, browser, answer))
            .map_err(|reason| browser.text(&reason));
        }
    }
    let requested_action = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let output_request = args.clone();
    call_selected(shared, session, tool, args)
        .await
        .map(|mut answer| {
            super::connection_notice::refused(shared, session, browser, &answer);
            super::receipt::stamp(shared, tool, &output_request, &mut answer["vwMcpResult"]);
            if tool == "chrome_status" {
                let addons = super::installation::addons(shared);
                answer["vwMcpResult"]["structuredContent"]["addons"] = addons.clone();
                if let Some(content) = answer["vwMcpResult"]["content"].as_array_mut() {
                    content
                        .push(json!({"type":"text", "text":json!({"addons":addons}).to_string()}));
                }
                super::output_status::compact(
                    shared,
                    session,
                    requested_action.as_deref(),
                    &mut answer["vwMcpResult"],
                );
                super::profile_agent::status(shared, session, &mut answer["vwMcpResult"]);
                super::output::guard_status(&mut answer["vwMcpResult"]);
            } else {
                super::profile_agent::failed_connection(
                    shared,
                    session,
                    tool,
                    &output_request,
                    &mut answer["vwMcpResult"],
                );
                super::profile_agent::chosen(
                    shared,
                    session,
                    tool,
                    &output_request,
                    &mut answer["vwMcpResult"],
                );
                if super::output_cache::observation(&answer["vwMcpResult"]) {
                    super::output_cache::bound(
                        shared,
                        session,
                        &output_request,
                        &mut answer["vwMcpResult"],
                    );
                }
                super::output::guard(&mut answer["vwMcpResult"]);
            }
            answer
        })
        .map_err(|reason| browser.text(&reason))
}

pub(super) async fn call_selected(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    validate_public_args(&args)?;
    if tool == "chrome_execute" {
        validate_execute_args(&args)?;
    } else if tool == "chrome_session" {
        validate_session_args(&args)?;
    }
    if shared.store.view(session).is_none() {
        return Err("Not run: this chat is not open in Vibe Work.".to_owned());
    }
    // Tool calls racing the interrupt keep the person's reason, including session get/cancel/stop.
    let browser = agent_browser::selected(shared, session, tool, &args);
    if tool != "chrome_status" {
        super::reconnect::stopped(shared, session, browser)?;
    }
    // A frozen receipt cannot revive an ended session. It only reads the app's existing buffer.
    if tool == "chrome_execute" && super::output_cache::is_continuation(&args) {
        return call_one(shared, session, tool, args).await;
    }
    run(shared, session, tool, args).await
}

async fn run(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    if tool != "chrome_execute" || args.get("steps").is_none() {
        return call_one(shared, session, tool, args).await;
    }
    validate_public_args(&args)?;
    let steps = super::batch::steps(&args)?;
    if let Err(reason) = validate_session(shared, session, tool, &args) {
        let browser = agent_browser::selected(shared, session, tool, &args);
        return lease_refusal(
            shared,
            browser,
            super::availability::live(shared, browser),
            reason,
        );
    }
    let id = args["sessionId"].clone();
    Ok(super::batch::run(
        steps,
        |mut step| {
            step["sessionId"] = id.clone();
            call_one(shared, session, tool, step)
        },
        || {
            shared
                .chrome_control
                .owner
                .control(json!({"action":"cancel","sessionId":id}));
        },
        super::batch_wait::Clock::default(),
    )
    .await)
}

fn local_reply(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: &Value,
    consent: Option<&super::read_consent::Consent>,
) -> Result<Option<Value>, String> {
    validate_public_args(args)?;
    super::output_file::validate(args)?;
    if tool == "chrome_execute"
        && let Some(action) = args["action"].as_str()
        && let Some(answer) = super::output_status::unknown_action(action)
    {
        return Ok(Some(json!({"vwMcpResult":answer})));
    }
    if shared.store.view(session).is_none() {
        return Err("Not run: this chat is not open in Vibe Work.".to_owned());
    }
    // The gateway's own checks decide; the card only words a refusal while nothing usable is connected
    // (`availability::refusal`). The person's switch and an outdated extension refuse first, even when live.
    let browser = agent_browser::selected(shared, session, tool, args);
    if tool != "chrome_status" {
        super::reconnect::stopped(shared, session, browser)?;
    }
    super::profile_agent::check(shared, session, tool, args)?;
    if let Some(refusal) = super::availability::refusal(shared, browser, true) {
        return Ok(Some(refusal));
    }
    super::check::authorize_read(shared, session, tool, args, consent)?;
    if let Err(reason) = validate_session(shared, session, tool, args) {
        return lease_refusal(
            shared,
            browser,
            super::availability::live(shared, browser),
            reason,
        )
        .map(Some);
    }
    if tool == "chrome_execute"
        && let Some(answer) = super::output_cache::resume(shared, session, args)
    {
        return answer.map(Some);
    }
    Ok(None)
}

pub(super) async fn call_one(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    let request = args.clone();
    let mut answer = call_one_inner(shared, session, tool, args, None, None).await?;
    super::receipt::stamp(shared, tool, &request, &mut answer["vwMcpResult"]);
    if tool != "chrome_status" {
        super::output::guard(&mut answer["vwMcpResult"]);
    }
    Ok(answer)
}

async fn wait_for_browser(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: &Value,
    browser: Browser,
    consent: Option<&super::read_consent::Consent>,
) -> Result<Option<Value>, String> {
    let acts = tool != "chrome_status"
        && !(tool == "chrome_session"
            && matches!(args["action"].as_str(), Some("get" | "cancel" | "stop")));
    if acts {
        if let Some(reply) = super::browser_start::wait(shared, session, browser).await? {
            return Ok(Some(reply));
        }
        if let Some(retry) = super::reconnect::wait(shared, session, browser).await? {
            return Ok(Some(retry));
        }
        // Waiting for a native port grants no authority: mode and the person's Stop are checked again.
        recheck(shared, session, browser, tool, args, consent)?;
    }
    Ok(None)
}

pub(super) async fn call_one_inner(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    mut args: Value,
    consent: Option<&super::read_consent::Consent>,
    previous: Option<&str>,
) -> Result<Value, String> {
    let opening = tool == "chrome_session" && args["action"] == "open";
    if opening {
        super::profile_seeding::ask_person(shared, session, &mut args).await?;
    }
    let _binding = if opening {
        Some(shared.chrome_control.binding_changes.lock().await)
    } else {
        None
    };
    if opening
        && let Some(profile) = args["profileId"].as_str()
        && ![
            "chrome",
            "edge",
            "safari",
            "firefox",
            vw_vocabulary::texts::CHROME_NAME,
            vw_vocabulary::texts::EDGE_NAME,
        ]
        .contains(&profile)
    {
        super::profile_bindings::check_agent(&shared.chrome_control, session, profile)?;
    }
    call_prepared(shared, session, tool, args, consent, previous).await
}

async fn call_prepared(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    mut args: Value,
    consent: Option<&super::read_consent::Consent>,
    previous: Option<&str>,
) -> Result<Value, String> {
    let browser = agent_browser::selected(shared, session, tool, &args);
    if tool == "chrome_execute" {
        super::reconnect::stopped(shared, session, browser)?;
        shared.browser_dialogs.check()?;
    }
    if let Some(reply) = local_reply(shared, session, tool, &args, consent)? {
        return Ok(reply);
    }
    if let Some(reply) = wait_for_browser(shared, session, tool, &args, browser, consent).await? {
        return Ok(reply);
    }
    let mut live = super::availability::live(shared, browser);
    if let Err(reason) = validate_session(shared, session, tool, &args) {
        return lease_refusal(shared, browser, live, reason);
    }
    if tool == "chrome_execute"
        && super::capabilities::gated(args["action"].as_str().unwrap_or_default())
    {
        // The live feature gate reads the helper's status for this call: its answer is the live connection.
        match super::capabilities::require(shared, &args).await {
            Ok(()) => live = true,
            Err(reason) => return lease_refusal(shared, browser, live, reason),
        }
    }
    // A status read can discover the reconnect itself. The local switch/version checks above still apply.
    if !(tool == "chrome_status" && browser.was_paired(shared))
        && let Some(refusal) = super::availability::gate(shared, browser, live)
    {
        return Ok(refusal);
    }
    if tool == "chrome_execute" {
        super::observation_support::require(shared, &args).await?;
        recheck(shared, session, browser, tool, &args, consent)?;
        validate_session(shared, session, tool, &args)?;
    }
    let request = tool == "chrome_session" && args["action"] == "open";
    if request {
        super::session_start::arguments(shared, session, browser, &mut args, consent).await?;
    }
    let pending = super::person_stop::PendingCall::new(shared, session, tool, &args);
    if super::output_file::requested(&args) {
        super::output_file::folder(shared, session)?;
    }
    if tool == "chrome_execute"
        && let Some(answer) = super::session_activity::before_execute(
            shared, session, browser, &args, &pending, consent,
        )
        .await?
    {
        return Ok(answer);
    }
    let clamped = super::output::clamp_request(tool, &mut args);
    let output_request = args.clone();
    if let Some(previous) = previous {
        args["_preflightOperationId"] = json!(previous);
    }
    let mut answer = super::output_file::send(shared, session, tool, args, consent)
        .await
        .map_err(|reason| pending.refusal(shared, session, browser).unwrap_or(reason))?;
    if request {
        answer = super::session_start::fresh(
            shared,
            session,
            browser,
            &pending,
            &output_request,
            answer,
        )
        .await?;
    }
    pending.finish(shared, session, browser, &answer)?;
    scope_result(
        shared,
        session,
        tool,
        request,
        browser,
        &pending,
        &mut answer,
    )?;
    if tool == "chrome_status" {
        super::page_plan_catalog::decorate(answer.value_mut(), output_request["action"].as_str());
    }
    super::check::annotate_modes(shared, session, tool, answer.value_mut());
    if let Some(reply) = super::availability::after_call(shared, browser, tool, request, &answer) {
        return Ok(reply);
    }
    Ok(super::output_cache::for_model(
        shared,
        session,
        &output_request,
        answer,
        tool,
        clamped,
    ))
}

/// Waiting for the adapter or a dialog never preserves an old browser-action authorization.
pub(super) fn recheck(
    shared: &Shared,
    session: &str,
    browser: Browser,
    tool: &str,
    args: &Value,
    consent: Option<&super::read_consent::Consent>,
) -> Result<(), String> {
    super::reconnect::stopped(shared, session, browser)?;
    super::check::authorize_read(shared, session, tool, args, consent)?;
    if tool == "chrome_execute" {
        shared.browser_dialogs.check()?;
    }
    Ok(())
}

pub(super) fn scope_result(
    shared: &Shared,
    session: &str,
    tool: &str,
    request: bool,
    browser: Browser,
    pending: &super::person_stop::PendingCall,
    answer: &mut owner::Answer,
) -> Result<(), String> {
    let mut tasks = shared
        .chrome_control
        .tasks
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    // Profile Off takes this same lease lock. Recheck every browser at admission so an off/on cycle
    // cannot grant a late open after the earlier reply validation passed.
    pending.finish(shared, session, browser, answer)?;
    let result = answer.value_mut();
    if request
        && result["isError"] != true
        && result["structuredContent"]["error"].is_null()
        && result["structuredContent"]["status"] != "unknown"
        && let Some(id) = result["structuredContent"]["sessionId"].as_str()
    {
        let profile = result["structuredContent"]["profileId"].as_str().ok_or("The browser did not identify the opened profile. Reconnect the profile before opening a session.")?;
        let work = shared
            .chrome_control
            .work
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if work
            .profile_of(id)
            .is_some_and(|old| !super::profile_ids::same(old, profile))
            || work.chat_of(id).is_some_and(|old| old != session)
        {
            return Err("This browser session is already associated with another chat or profile. Reconnect this profile before opening a new session.".into());
        }
        super::profile_bindings::remember(&shared.chrome_control, session, profile, Some(id))?;
        tasks.retain(|id, owner| {
            owner != session
                || work
                    .profile_of(id)
                    .is_some_and(|profile| Browser::from_profile(profile) != browser)
        });
        drop(work);
        tasks.insert(id.to_owned(), session.to_owned());
        shared
            .chrome_control
            .work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .open(
                id,
                session,
                result["structuredContent"]["state"]
                    .as_str()
                    .unwrap_or("disconnected"),
                result["structuredContent"]["profileId"]
                    .as_str()
                    .unwrap_or(match browser {
                        Browser::Chrome => "chrome:",
                        Browser::Edge => "edge:",
                        Browser::Safari => "safari:",
                        Browser::Firefox => "firefox:",
                    }),
                pending.turn.as_deref(),
            );
    }
    if tool == "chrome_status" && result["structuredContent"].get("profiles").is_some() {
        for key in ["sessions", "pending"] {
            if let Some(rows) = result["structuredContent"][key].as_array_mut() {
                rows.retain(|row| {
                    row["sessionId"]
                        .as_str()
                        .is_some_and(|id| tasks.get(id).map(String::as_str) == Some(session))
                });
            }
        }
        result["content"] = json!([{"type":"text","text":serde_json::to_string(&result["structuredContent"]).unwrap_or_default()}]);
    }
    drop(tasks);
    if request {
        owner::publish(shared);
    }
    Ok(())
}

#[cfg(test)]
mod listing_tests {
    use serde_json::json;

    #[tokio::test]
    async fn missing_chrome_session_fields_are_named_before_any_helper_call() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        for (args, field) in [
            (json!({}), "action"),
            (json!({"action":"open"}), "profileId"),
            (json!({"action":"open","profileId":""}), "profileId"),
            (json!({"action":"get"}), "sessionId"),
            (json!({"action":"cancel"}), "sessionId"),
            (json!({"action":"stop","sessionId":""}), "sessionId"),
        ] {
            let reason = super::call(&fixture.core.shared, "s", "chrome_session", args)
                .await
                .unwrap_err();
            assert!(reason.contains(field), "missing {field}: {reason}");
        }
        fixture.finish().await;
    }

    #[tokio::test]
    async fn malformed_execute_input_is_refused_before_reopening_a_chrome_session() {
        assert!(
            super::validate_execute_args(&json!({"sessionId":"t","action":"page.read","args":{}}))
                .is_ok()
        );
        assert!(
            super::validate_execute_args(
                &json!({"sessionId":"t","steps":[{"action":"page.read","args":{}}]})
            )
            .is_ok()
        );
        let core = crate::offline_core::start_with("chrome-input-shape", |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
            config.chrome_control.user_home = Some(dir.join("chrome-home"));
        })
        .await;
        for args in [
            json!({"sessionId":"unopened","action":"page.read"}),
            json!({"sessionId":"unopened","action":"page.read","args":null}),
            json!({"sessionId":"unopened","action":"page.read","args":{},"steps":[]}),
        ] {
            let reason = super::call(&core.shared, "s", "chrome_execute", args)
                .await
                .unwrap_err();
            assert!(
                reason.contains("action and args") || reason.contains("batch"),
                "{reason}"
            );
        }
        core.shutdown().await;
    }

    /// Vibe lists a session's MCP tools once, at session start, often before the core has the session's view or
    /// mode, and keeps the answer for up to a day for every chat whose tool server has the same launch line.
    /// Before the change the list was empty for a known Plan/Explore chat or with the Chrome
    /// switch off, so such a chat seeded an empty Chrome list that later chats got from Vibe's cache. After it the
    /// list depends only on the package; the launch line carries the chat's choice (`agent_tools::chrome_launch`).
    #[tokio::test]
    async fn the_tool_list_is_the_same_for_every_chat_and_switch_state() {
        let absent = crate::offline_core::start("chrome-listing-absent").await;
        assert_eq!(
            super::list(&absent.shared),
            json!([]),
            "no package: no Chrome tools"
        );
        absent.shutdown().await;
        let core = crate::offline_core::start_with("chrome-listing", |config, dir| {
            // Its own home: the enabled switch below is saved in the Chrome state folder, never the real home.
            config.chrome_control.package = Some(dir.join("package"));
            config.chrome_control.user_home = Some(dir.join("chrome-home"));
        })
        .await;
        let shared = &core.shared;
        let full = json!(vw_agent_tools::chrome_tool_list());
        assert_ne!(full, json!([]));
        assert_eq!(super::list(shared), full, "a session still starting");
        let _ = shared
            .store
            .load(
                json!({"session":{"id":"s"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        for enabled in [true, false] {
            shared.chrome_control.set_enabled(enabled).unwrap();
            shared
                .chrome_control
                .set_browser_enabled(super::super::agent_browser::Browser::Edge, enabled)
                .unwrap();
            for mode in ["ask", "auto-approve", "vw-plan", "explore", "unknown"] {
                shared.access.observe_request(
                    "session/agent/update",
                    "s",
                    &json!({"agentName":mode}),
                );
                assert_eq!(super::list(shared), full, "{mode} {enabled}");
                let refused =
                    super::super::check::authorize(shared, "s", "chrome_status", &json!({}));
                assert_eq!(
                    refused.is_err(),
                    !enabled || matches!(mode, "vw-plan" | "explore" | "unknown"),
                    "{mode} {enabled}: {refused:?}"
                );
            }
        }
        core.shutdown().await;
    }
}

#[cfg(test)]
#[path = "gateway_own_failure_tests.rs"]
mod own_failure_tests;
