//! Session-authenticated Chrome tools. Each call uses the current chat mode.
use super::{
    agent_browser::{self, Browser},
    owner,
};
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::Arc;

#[path = "gateway_args.rs"]
mod args;
#[path = "gateway_ownership.rs"]
mod ownership;
#[path = "gateway_scope.rs"]
mod scope;

pub(crate) use args::validate_execute_args;
pub(super) use args::{validate_public_args, validate_session_args};
use ownership::lease_refusal;
#[path = "gateway_steps.rs"]
mod steps;
pub(super) use ownership::validate_session;
pub(super) use scope::{halted, recheck, scope_result};
use steps::{confirm_open_answer, prepare_frame, wait_for_browser};

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

pub(crate) async fn call(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    super::deadline::run(call_entry(shared, session, tool, args)).await
}

async fn call_entry(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    super::firefox_sharing::refuse_agent_reach(tool, &args)?;
    let browser = agent_browser::selected(shared, session, tool, &args);
    // Stop points of one call (A2-02): `chrome_execute` here, before anything else answers; `chrome_session` in
    // `call_selected` after its arguments are checked; both again at every point where the call resumes after a
    // wait (`call_prepared`, `recheck`); the reply is admitted by `PendingCall::finish`.
    if tool == "chrome_execute" {
        halted(shared, session, browser, tool)?;
    }
    if matches!(tool, "chrome_status" | "chrome_execute")
        && let Some(action) = args["action"].as_str()
        && let Some(answer) = super::output_status::unknown_action(action)
    {
        validate_public_args(&args, browser)?;
        let mut answer = answer;
        super::receipt::stamp(shared, tool, &args, &mut answer);
        super::output::guard(&mut answer);
        return Ok(json!({"vwMcpResult":answer}));
    }
    if tool == "chrome_status" {
        validate_public_args(&args, browser)?;
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
                crate::paused_tools_gate::note_resumed_helper(
                    shared,
                    session,
                    &mut answer["vwMcpResult"],
                );
            }
            answer
        })
        .map_err(|reason| browser.text(&reason))
        .inspect(|_| {
            if tool != "chrome_status" {
                super::frames::sync_chat(shared, session);
            }
        })
}

pub(super) async fn call_selected(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    let browser = agent_browser::selected(shared, session, tool, &args);
    validate_public_args(&args, browser)?;
    if tool == "chrome_execute" {
        validate_execute_args(&args, browser)?;
    } else if tool == "chrome_session" {
        validate_session_args(&args, browser)?;
    }
    if shared.store.view(session).is_none() {
        return Err("Not run: this chat is not open in Vibe Work. Open the chat in Vibe Work, then try again.".to_owned());
    }
    // Tool calls racing the interrupt keep the person's reason, including session get/cancel/stop.
    // `chrome_execute` was checked on entry (`call_entry`, the only caller with that tool) with no wait since.
    if !matches!(tool, "chrome_status" | "chrome_execute") {
        halted(shared, session, browser, tool)?;
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
    let browser = agent_browser::selected(shared, session, tool, &args);
    validate_public_args(&args, browser)?;
    let steps = super::batch::steps(&args)?;
    if let Err(reason) = validate_session(shared, session, tool, &args) {
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
    let browser = agent_browser::selected(shared, session, tool, args);
    validate_public_args(args, browser)?;
    super::output_file::validate(args)?;
    if tool == "chrome_execute"
        && let Some(action) = args["action"].as_str()
        && let Some(answer) = super::output_status::unknown_action(action)
    {
        return Ok(Some(json!({"vwMcpResult":answer})));
    }
    if shared.store.view(session).is_none() {
        return Err("Not run: this chat is not open in Vibe Work. Open the chat in Vibe Work, then try again.".to_owned());
    }
    // The gateway's own checks decide; the card only words a refusal while nothing usable is connected
    // (`availability::refusal`). The person's switch and an outdated extension refuse first, even when live.
    // `chrome_execute` was checked by `call_prepared` just before (its only caller), with no wait since.
    if !matches!(tool, "chrome_status" | "chrome_execute") {
        halted(shared, session, browser, tool)?;
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
    if tool == "chrome_execute" {
        // References this read returned belong to the person's browser, never to the built-in one.
        shared.browser_ref_origins.note(
            session,
            Some(agent_browser::selected(shared, session, tool, &request)),
            super::output_cache::read_refs(shared, session, &answer["vwMcpResult"]),
        );
    }
    super::receipt::stamp(shared, tool, &request, &mut answer["vwMcpResult"]);
    if tool != "chrome_status" {
        super::output::guard(&mut answer["vwMcpResult"]);
    }
    Ok(answer)
}

pub(super) async fn call_one_inner(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
    consent: Option<&super::read_consent::Consent>,
    previous: Option<&str>,
) -> Result<Value, String> {
    super::deadline::run(call_one_scoped(
        shared, session, tool, args, consent, previous,
    ))
    .await
}

async fn call_one_scoped(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    mut args: Value,
    consent: Option<&super::read_consent::Consent>,
    previous: Option<&str>,
) -> Result<Value, String> {
    // A pause that started after this call's receipt: wait inside the tool's own deadline, before any shared lock
    // (another chat's open must not wait for this chat's person). Every check after this runs after the wait.
    if crate::paused_tools_gate::browser_call(tool, &args) {
        crate::paused_tools_gate::wait(
            shared,
            session,
            super::deadline::remaining(crate::paused_tools::GATEWAY_WAIT),
        )
        .await?;
    }
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
    // The call resumes here after the pause wait and the profile card.
    if tool == "chrome_execute" {
        halted(shared, session, browser, tool)?;
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
        // Again on the final arguments: a shorthand or remembered choice can resolve to the person's Firefox.
        super::firefox_sharing::refuse_agent_reach(tool, &args)?;
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
    prepare_frame(shared, session, tool, browser, &args, consent)
        .await
        .map_err(|reason| pending.refusal(shared, session, browser).unwrap_or(reason))?;
    let mut answer = super::output_file::send(shared, session, tool, args, consent)
        .await
        .map_err(|reason| pending.refusal(shared, session, browser).unwrap_or(reason))?;
    super::person_stop::read_stop_behind(shared, tool, &answer).await;
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
        confirm_open_answer(shared, session, &output_request, &answer)?;
    }
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

#[cfg(test)]
#[path = "gateway_listing_tests.rs"]
mod listing_tests;

#[cfg(test)]
#[path = "gateway_own_failure_tests.rs"]
mod own_failure_tests;

#[cfg(test)]
#[path = "gateway_deadline_tests.rs"]
mod deadline_tests;
