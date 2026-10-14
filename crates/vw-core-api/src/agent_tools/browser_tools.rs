//! `browser_read` / `browser_act` behind a session credential: the session's own built-in browser only.
//! `target` keeps the routed shape for the user's own Chrome: any value other than `builtin` is refused
//! with a clear message.
//!
//! Depends on: `browser` (driver, navigation), vw-browser, vw-agent-tools (`wire`). Called by: `tools::call`.
//! `browser_act` `fill` goes to `browser_fill` (a vault login into a sign-in field) before the driver.
//! A read that searches for a registered secret is refused before accessing the page, with no location information.
//! Invariants: the agent's navigations are strict (proven local ports) and public pages need the mount's proven
//! network guard; an answer with a screenshot carries its PNG under `wire::IMAGE_KEY` (<= 1 MiB before base64).

use std::sync::Arc;

use serde_json::{Value, json};
use vw_agent_tools::wire::{self, tools as names};
use vw_browser::Actor;

use crate::browser::{CoreNav, browser, read_options};
use crate::core::Shared;

fn target(args: &Value) -> Result<(), String> {
    match args.get("target").and_then(Value::as_str) {
        None | Some("builtin") => Ok(()),
        Some("chrome") => Err(
            "target 'chrome' (your own Chrome) is not available yet: use target 'builtin' (the default), the \
             browser in Vibe Work next to the chat"
                .to_owned(),
        ),
        Some(other) => Err(format!(
            "unknown target '{}': use 'builtin'",
            other.chars().take(20).collect::<String>()
        )),
    }
}

fn check_current(shared: &Shared, owner: &str, page: &str) -> Result<(), String> {
    let hub = crate::previews::hub(shared).map_err(|error| error.message)?;
    if shared.store.view(owner).is_none()
        || agent_stopped(&hub.browser, &hub.service, owner)
        || !crate::browser_tabs::agent_page_allowed(&hub.service, owner, page)
    {
        return Err("browser tab is no longer available to this agent".to_owned());
    }
    Ok(())
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

/// Runs one browser tool for `session_id`.
#[allow(clippy::too_many_lines)]
pub(crate) async fn call(
    shared: &Arc<Shared>,
    session_id: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    if tool == names::BROWSER_CAPTURE_SECRET {
        return Err("Not run: saving a key to the vault is the user's action. Ask the user to save it in the browser panel (select the key, then Save to vault).".to_owned());
    }
    if tool == names::BROWSER_READ
        && args
            .get("find")
            .and_then(Value::as_str)
            .is_some_and(|query| vw_secrets::find(query.as_bytes()).is_some())
    {
        return Err(
            "Not run: searching for a stored secret is not allowed. Use its label or capture ref instead."
                .to_owned(),
        );
    }
    target(&args)?;
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
    match tool {
        names::BROWSER_READ => {
            let options = read_options(
                args.get("maxRefs").and_then(Value::as_u64),
                args.get("textPage").and_then(Value::as_u64),
                args.get("maxBytes").and_then(Value::as_u64),
                args.get("find").and_then(Value::as_str),
            );
            let screenshot = args
                .get("screenshot")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mut canceled = driver.sessions().cancel_signal(&page);
            let (mut observation, shot) = tokio::select! {
                biased;
                _ = canceled.changed() => return Err("browser tab action was canceled".to_owned()),
                answer = driver.read(&page, options, screenshot, Actor::Agent) => answer?,
            };
            check_current(shared, session_id, &page)?;
            attach_tab(shared, &page, &mut observation);
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
                observation[wire::IMAGE_KEY] =
                    json!({"data": shot.base64(), "mimeType": "image/png"});
            }
            Ok(observation)
        }
        names::BROWSER_ACT if args.get("action").and_then(Value::as_str) == Some("fill") => {
            crate::agent_access::check(shared)?;
            let mut args = args;
            if let Value::Object(map) = &mut args {
                map.remove("tab");
            }
            // A vault login into a sign-in field (the core's path: checks, grant, password, write).
            let mut receipt = crate::browser_fill::fill(
                shared,
                driver,
                &page,
                &args,
                Actor::Agent,
                vw_gate::Requester::Agent,
            )
            .await?;
            check_current(shared, session_id, &page)?;
            attach_tab(shared, &page, &mut receipt);
            vw_browser::observe::fit(&mut receipt, vw_browser::observe::READ_BUDGET);
            Ok(receipt)
        }
        names::BROWSER_ACT => {
            let mut args = args;
            if let Value::Object(map) = &mut args {
                map.remove("target");
                map.remove("tab");
            }
            let action = vw_browser::parse(&args)?;
            let nav = CoreNav {
                shared: shared.clone(),
                strict: true,
            };
            let mut receipt = driver
                .act(&page, action, Actor::Agent, &nav)
                .await
                .map_err(|e| e.text)?;
            // The act already completed. Its own navigation may have stopped the page;
            // preserve what happened instead of turning sent input into a refusal.
            attach_tab(shared, &page, &mut receipt);
            vw_browser::observe::fit(&mut receipt, vw_browser::observe::READ_BUDGET);
            Ok(receipt)
        }
        names::BROWSER_CAPTURE => {
            let mut args = args;
            if let Value::Object(map) = &mut args {
                map.remove("target");
                map.remove("tab");
            }
            let mut canceled = driver.sessions().cancel_signal(&page);
            let mut capture = tokio::select! {
                biased;
                _ = canceled.changed() => return Err("browser tab action was canceled".to_owned()),
                answer = crate::browser_capture::run(shared, &page, &args, Actor::Agent) =>
                    answer.map_err(|error| error.text)?,
            };
            check_current(shared, session_id, &page)?;
            attach_tab(shared, &page, &mut capture);
            Ok(capture)
        }
        _ => Err("unknown tool".to_owned()),
    }
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
