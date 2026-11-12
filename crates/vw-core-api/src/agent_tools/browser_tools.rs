//! `browser_read` / `browser_act` behind a session credential: the session's own built-in browser only.
//! Only the built-in target runs here; old Chrome requests point to the separate Chrome tools.
//!
//! Depends on: `browser` (driver, navigation), vw-browser, vw-agent-tools (`wire`). Called by: `tools::call`.
//! `browser_act` `fill` goes to `browser_fill` (a vault login into a sign-in field) before the driver.
//! A read that searches for a registered secret is refused before accessing the page, with no location information.
//! Invariants: local and public addresses share the browser URL policy; agent calls retain refusal reporting and
//! pinned App preview port ownership checks. Screenshots carry their PNG under `wire::IMAGE_KEY` (<= 1 MiB before base64).

use std::sync::Arc;

use serde_json::{Value, json};
use vw_agent_tools::wire::{self, tools as names};
use vw_browser::Actor;

use crate::browser::{CoreNav, browser, read_options};
use crate::core::Shared;

fn searches_stored_secret(args: &Value) -> bool {
    fn contains(value: &Value) -> bool {
        match value {
            Value::String(text) => vw_secrets::find(text.as_bytes()).is_some(),
            Value::Array(items) => items.iter().any(contains),
            Value::Object(fields) => fields.values().any(contains),
            _ => false,
        }
    }
    ["find", "selector", "extract"]
        .iter()
        .filter_map(|key| args.get(*key))
        .any(contains)
}

fn target(chrome_available: bool, args: &Value) -> Result<(), String> {
    if args["target"] == "chrome" && !chrome_available {
        return Err("This version works in Vibe Work's own browser.".to_owned());
    }
    match args.get("target").and_then(Value::as_str) {
        None | Some("builtin") => Ok(()),
        Some("chrome") => Err("This tool operates only in Vibe Work's browser.".to_owned()),
        Some(other) => Err(format!(
            "unknown target '{}': use 'builtin'",
            other.chars().take(20).collect::<String>()
        )),
    }
}

pub(crate) fn check_current(shared: &Shared, owner: &str, page: &str) -> Result<(), String> {
    check_agent(shared, owner)?;
    shared.browser_dialogs.check()?;
    let hub = crate::previews::hub(shared).map_err(|error| error.message)?;
    if shared.store.view(owner).is_none()
        || agent_stopped(&hub.browser, &hub.service, owner)
        || !crate::browser_tabs::agent_page_allowed(&hub.service, owner, page)
    {
        return Err("browser tab is no longer available to this agent".to_owned());
    }
    Ok(())
}

/// A named modal must not invite a retry after the person has stopped this browser.
fn check_agent(shared: &Shared, owner: &str) -> Result<(), String> {
    crate::browser_settings::check(shared)?;
    if let Some(hub) = &shared.preview
        && agent_stopped(&hub.browser, &hub.service, owner)
    {
        return Err(vw_browser::STOPPED.to_owned());
    }
    crate::browser_handover::check(shared, owner)
}

fn agent_stopped(
    driver: &vw_browser::Browser,
    service: &vw_preview::PreviewService,
    owner: &str,
) -> bool {
    driver.sessions().stopped(owner)
        || service
            .web_tabs(owner)
            .iter()
            .any(|page| driver.sessions().stopped(page))
}

/// Checks consent before the MCP call starts; the returned action has no person-wait.
pub(super) async fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    args: &Value,
) -> Result<Option<crate::ask::tool_calls::receipts::Execution>, String> {
    check_agent(shared, owner)?;
    shared.browser_dialogs.check()?;
    vw_agent_tools::browser_input::validate(names::BROWSER_ACT, args)?;
    if args["action"] == "form" {
        vw_browser::parse_form(args)?;
    }
    target(shared.chrome_control.config.available(), args)?;
    if args.get("steps").is_some() {
        return super::browser_batch::prepare(shared, owner, source, call, args)
            .await
            .map(Some);
    }
    if matches!(
        args["action"].as_str(),
        Some(
            "click"
                | "check"
                | "uncheck"
                | "page.click"
                | "page.double_click"
                | "page.right_click"
                | "page.middle_click"
                | "page.triple_click"
        )
    ) && crate::chrome_control::check::mode(shared, owner).as_deref() == Some("ask")
    {
        return super::browser_sending::prepare(shared, owner, source, call, args)
            .await
            .map(Some);
    }
    if args["action"]
        .as_str()
        .is_some_and(|action| action.starts_with("window."))
    {
        return crate::browser_windows::prepare(shared, owner, args).map(Some);
    }
    if args["action"] == "close_tab" {
        return crate::browser_tab_tools::prepare_close(
            shared,
            owner,
            args["tab"].as_str().unwrap_or_default(),
        )
        .map(Some);
    }
    if args["action"] != "fill" && args["action"] != "auth.basic" {
        return Ok(None);
    }
    let driver = browser(shared).map_err(|e| e.message)?;
    let hub = crate::previews::hub(shared).map_err(|e| e.message)?;
    let page = crate::browser_tabs::agent_page(&hub.service, owner, args["tab"].as_str())
        .map_err(|e| e.message)?;
    crate::browser_tabs::front_page(&hub.service, owner, &page)
        .await
        .map_err(|e| e.message)?;
    let mut input = args.clone();
    if let Some(input) = input.as_object_mut() {
        input.remove("tab");
    }
    let action = if args["action"] == "auth.basic" {
        crate::browser_auth::prepare(
            shared,
            &page,
            &input,
            Actor::Agent,
            vw_gate::Requester::Agent,
        )
        .await?
    } else {
        crate::browser_fill::prepare(
            shared,
            driver,
            &page,
            &input,
            Actor::Agent,
            vw_gate::Requester::Agent,
        )
        .await?
    };
    let shared = Arc::downgrade(shared);
    let owner = owner.to_owned();
    Ok(Some(Box::pin(async move {
        let shared = shared
            .upgrade()
            .ok_or(crate::browser_outcomes::SESSION_UNAVAILABLE)?;
        let mut receipt = action.await?;
        check_current(&shared, &owner, &page)?;
        attach_tab(&shared, &page, &mut receipt);
        vw_browser::observe::fit(&mut receipt, vw_browser::observe::READ_BUDGET);
        Ok(receipt)
    })))
}

/// Runs one browser tool for `session_id`.
pub(crate) async fn call(
    shared: &Arc<Shared>,
    session_id: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    let allow_send = crate::chrome_control::check::mode(shared, session_id).as_deref()
        != Some("ask")
        || crate::chrome_control::builtin::ask_required(&args)
            .is_some_and(|tier| tier > vw_ask::chrome::Tier::Ask);
    call_with_sending(shared, session_id, tool, args, allow_send).await
}

pub(super) async fn call_with_sending(
    shared: &Arc<Shared>,
    session_id: &str,
    tool: &str,
    args: Value,
    allow_send: bool,
) -> Result<Value, String> {
    validate_call(shared, session_id, tool, &args)?;
    let tab = match args.get("tab") {
        None => None,
        Some(Value::String(tab)) if !tab.trim().is_empty() => Some(tab.as_str()),
        Some(_) => return Err("tab must be a nonempty web tab id or 'new'".to_owned()),
    };
    let driver = browser(shared).map_err(|e| e.message)?;
    if tool == names::BROWSER_ACT
        && matches!(
            args.get("action").and_then(Value::as_str),
            Some("list_tabs" | "close_tab")
        )
    {
        return crate::browser_tab_tools::call(shared, session_id, &args, tab).await;
    }
    if agent_stopped(
        driver,
        &crate::previews::hub(shared).map_err(|e| e.message)?.service,
        session_id,
    ) {
        return Err(vw_browser::STOPPED.to_owned());
    }
    let page = crate::browser_tabs::agent_page(
        &crate::previews::hub(shared)
            .map_err(|error| error.message)?
            .service,
        session_id,
        tab,
    )
    .map_err(|error| error.message)?;
    crate::browser_tabs::front_page(
        &crate::previews::hub(shared).map_err(|e| e.message)?.service,
        session_id,
        &page,
    )
    .await
    .map_err(|e| e.message)?;
    if tool == names::BROWSER_ACT
        && matches!(
            args["action"].as_str(),
            Some(
                "clipboard.read"
                    | "clipboard.write"
                    | "clipboard.read_rich"
                    | "clipboard.write_rich"
            )
        )
    {
        return clipboard_action(shared, session_id, &page, driver, &args).await;
    }
    if tool == names::BROWSER_READ
        && (["extract", "saveTo", "appendFrom", "deduplicate"]
            .iter()
            .any(|key| args.get(*key).is_some()))
    {
        return crate::browser_data::read(shared, session_id, &page, &args, driver).await;
    }
    match tool {
        names::BROWSER_READ => read_page(shared, session_id, &page, &args, driver).await,
        names::BROWSER_ACT => {
            act_on_page(shared, session_id, &page, driver, args, allow_send).await
        }
        names::BROWSER_CAPTURE => capture_page(shared, session_id, &page, driver, args).await,
        _ => Err("unknown tool".to_owned()),
    }
}

fn validate_call(
    shared: &Shared,
    session_id: &str,
    tool: &str,
    args: &Value,
) -> Result<(), String> {
    check_agent(shared, session_id)?;
    vw_agent_tools::browser_input::validate(tool, args)?;
    if tool == names::BROWSER_ACT {
        shared.browser_dialogs.check()?;
    }
    if args.get("steps").is_some() {
        return Err(
            "This batch needs its pre-tool hook decisions; retry the call through its hook.".into(),
        );
    }
    #[cfg(feature = "agent-vault")]
    if tool == names::BROWSER_CAPTURE_SECRET {
        return Err(
            "This vault save needs its pre-tool hook decision; retry the tool through its hook."
                .into(),
        );
    }
    #[cfg(not(feature = "agent-vault"))]
    if tool == names::BROWSER_CAPTURE_SECRET {
        return Err("Not run: saving a key to the vault is the user's action. Ask the user to select the key in the browser panel, then choose Save to vault in the toolbar, in Browser options when narrow, or by right-click on the selection.".to_owned());
    }
    if tool == names::BROWSER_READ && searches_stored_secret(args) {
        return Err(
            "Not run: searching for a stored secret is not allowed. Use its label or capture ref instead."
                .to_owned(),
        );
    }
    target(shared.chrome_control.config.available(), args)?;
    if tool == names::BROWSER_ACT {
        crate::chrome_control::builtin::check_full(
            shared.access.remembered(session_id).as_deref(),
            args,
        )?;
    }
    Ok(())
}

async fn clipboard_action(
    shared: &Arc<Shared>,
    session_id: &str,
    page: &str,
    driver: &vw_browser::Browser,
    args: &Value,
) -> Result<Value, String> {
    check_current(shared, session_id, page)?;
    let mut canceled = driver.sessions().cancel_signal(page);
    let mut answer = tokio::select! {
        biased;
        _=canceled.changed()=>return Err("Clipboard action canceled; start a new browser action.".into()),
        result=crate::browser_clipboard::run(shared,args)=>result?,
    };
    check_current(shared, session_id, page)?;
    attach_tab(shared, page, &mut answer);
    Ok(answer)
}

async fn act_on_page(
    shared: &Arc<Shared>,
    session_id: &str,
    page: &str,
    driver: &vw_browser::Browser,
    args: Value,
    allow_send: bool,
) -> Result<Value, String> {
    if args["action"] == "form" {
        let nav = CoreNav {
            shared: shared.clone(),
            strict: true,
        };
        let mut receipt = driver
            .fill_form(page, &args, Actor::Agent, &nav, allow_send)
            .await?;
        attach_tab(shared, page, &mut receipt);
        return Ok(receipt);
    }
    if args["action"] == "auth.basic" {
        return Err("HTTP sign-in needs its vault decision before the tool starts; retry through the tool hook.".into());
    }
    let mut args = args;
    if let Value::Object(map) = &mut args {
        map.remove("target");
        map.remove("tab");
    }
    let action = crate::browser_upload::load(shared, session_id, vw_browser::parse(&args)?).await?;
    let nav = CoreNav {
        shared: shared.clone(),
        strict: true,
    };
    check_agent(shared, session_id)?;
    shared.browser_dialogs.check()?;
    let mut receipt = driver
        .act_with_sending(page, action, Actor::Agent, &nav, allow_send)
        .await
        .map_err(|e| e.text)?;
    crate::browser_recording::save(shared, session_id, &mut receipt)?;
    // The act already completed. Its own navigation may have stopped the page;
    // preserve what happened instead of turning sent input into a refusal.
    attach_tab(shared, page, &mut receipt);
    let image = receipt
        .as_object_mut()
        .and_then(|o| o.remove(wire::IMAGE_KEY));
    vw_browser::observe::fit(&mut receipt, vw_browser::observe::READ_BUDGET);
    if let Some(image) = image {
        receipt[wire::IMAGE_KEY] = image;
    }
    Ok(receipt)
}

async fn capture_page(
    shared: &Arc<Shared>,
    session_id: &str,
    page: &str,
    driver: &vw_browser::Browser,
    args: Value,
) -> Result<Value, String> {
    let mut args = args;
    if let Value::Object(map) = &mut args {
        map.remove("target");
        map.remove("tab");
    }
    let mut canceled = driver.sessions().cancel_signal(page);
    let mut capture = tokio::select! {
        biased;
        _ = canceled.changed() => return Err("browser tab action was canceled".to_owned()),
        answer = crate::browser_capture::run(shared, page, &args, Actor::Agent) =>
            answer.map_err(|error| error.text)?,
    };
    check_current(shared, session_id, page)?;
    attach_tab(shared, page, &mut capture);
    Ok(capture)
}

pub(crate) fn attach_tab(shared: &Shared, page: &str, answer: &mut Value) {
    if let Some(view) = shared
        .preview
        .as_ref()
        .and_then(|hub| hub.service.view(page))
    {
        let site = view
            .url
            .as_deref()
            .and_then(|url| url::Url::parse(url).ok())
            .and_then(|url| url.host_str().map(str::to_owned));
        let title = view.title.as_deref().map(|value| {
            value
                .chars()
                .take(vw_browser::observe::ITEM_CHARS)
                .collect::<String>()
        });
        let url = view
            .url
            .as_deref()
            .map(|value| value.chars().take(300).collect::<String>());
        answer["tab"] = json!({"id": page, "site": site});
        answer["untrustedPageContent"]["tab"] = json!({"title": title, "url": url});
    } else {
        answer["tab"] = json!({"id": page, "site": null});
        answer["untrustedPageContent"]["tab"] = json!({"title": null, "url": null});
    }
}

/// Reads one admitted page, retaining cancellation and the same observation/image budget.
async fn read_page(
    shared: &Arc<Shared>,
    session_id: &str,
    page: &str,
    args: &Value,
    driver: &vw_browser::Browser,
) -> Result<Value, String> {
    let mut options = read_options(
        args.get("maxRefs").and_then(Value::as_u64),
        args.get("textPage").and_then(Value::as_u64),
        args.get("maxBytes").and_then(Value::as_u64),
        args.get("find").and_then(Value::as_str),
    );
    if let Some(selector) = args.get("selector") {
        options.selector = Some(serde_json::from_value(selector.clone()).map_err(|_| "A selector needs kind (css, xpath, alt, title or testId), value and optional index.")?);
    }
    options.root_ref = args.get("ref").and_then(Value::as_str).map(str::to_owned);
    if let Some(cursor) = args.get("textCursor") {
        if options.find.is_some() || options.text_page.is_some() {
            return Err("textCursor continues plain text; use it without find or textPage".into());
        }
        let cursor: vw_browser::TextCursor = serde_json::from_value(cursor.clone())
            .map_err(|_| "textCursor needs nonnegative page and offset integers")?;
        if cursor.page > 10_000 || cursor.offset > vw_browser::model::MAX_ANSWER {
            return Err("textCursor exceeds the text paging limit".into());
        }
        options.text_cursor = Some(cursor);
    }
    if let Some(cursor) = args.get("choiceCursor") {
        let cursor: vw_browser::ChoiceCursor = serde_json::from_value(cursor.clone())
            .map_err(|_| "choiceCursor needs nonnegative frame and offset integers")?;
        if cursor.frame >= 64 || cursor.offset > vw_browser::model::MAX_REFS {
            return Err(
                "choiceCursor exceeds the observed choice limit; use find for a label".into(),
            );
        }
        options.choice_cursor = Some(cursor);
    }
    let screenshot = args
        .get("screenshot")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut canceled = driver.sessions().cancel_signal(page);
    let (mut observation, shot) = tokio::select! {
        biased;
        _ = canceled.changed() => return Err("browser tab action was canceled".to_owned()),
        answer = driver.read(page, options, screenshot, Actor::Agent) => answer?,
    };
    check_current(shared, session_id, page)?;
    attach_tab(shared, page, &mut observation);
    vw_browser::observe::fit(
        &mut observation,
        args.get("maxBytes")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(vw_browser::observe::READ_BUDGET)
            .clamp(
                vw_browser::observe::MIN_BUDGET,
                vw_browser::observe::MAX_BUDGET,
            ),
    );
    if let Some(shot) = shot {
        observation[wire::IMAGE_KEY] = json!({"data": shot.base64(), "mimeType": "image/png"});
    }
    Ok(observation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_cannot_search_for_stored_values() {
        let value = "SelectorPrivacyFixtureOnly12345";
        vw_secrets::register(vw_secrets::SecretKind::VaultValue, value);
        assert!(searches_stored_secret(
            &json!({"selector":{"kind":"css","value":format!("input[value='{value}']")}})
        ));
        assert!(searches_stored_secret(&json!({"find":value})));
        assert!(searches_stored_secret(
            &json!({"extract":{"kind":"list","selector":format!("[data-value='{value}']")}})
        ));
        assert!(!searches_stored_secret(
            &json!({"selector":{"kind":"testId","value":"password"}})
        ));
    }

    #[test]
    fn legacy_chrome_target_points_to_the_users_chrome_tools() {
        assert_eq!(
            target(true, &json!({"target": "chrome"})),
            Err("This tool operates only in Vibe Work's browser.".to_owned())
        );
    }
}
