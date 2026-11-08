//! App-owned launch of a paired, closed browser before any action is sent. The pre-tool hook supplies Ask consent;
//! the gateway waits for the real helper connection. One launch is shared by concurrent chats, never their windows.
use super::{AddonState, agent_browser::Browser};
use crate::{ask::tool_calls, core::Shared};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);

enum Attempt {
    Pending(u64),
    Finished(Result<(), String>),
}
#[derive(Default)]
struct State {
    next: u64,
    attempts: [Option<Attempt>; 2],
}
/// A failed or timed-out launch is not repeated until a live connection is observed.
#[derive(Default)]
pub(super) struct Attempts(Mutex<State>);
impl Attempts {
    pub(super) fn connected(&self, shared: &Shared) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for browser in [Browser::Chrome, Browser::Edge] {
            if super::availability::live(shared, browser) {
                state.attempts[usize::from(browser == Browser::Edge)] = None;
            }
        }
    }
    pub(super) fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attempts = [None, None];
    }
    fn pending(&self, browser: Browser) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attempts[usize::from(browser == Browser::Edge)]
        .is_some()
    }
    fn begin(&self, shared: &Shared, browser: Browser) -> Option<u64> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let index = usize::from(browser == Browser::Edge);
        if state.attempts[index].is_some() || super::availability::live(shared, browser) {
            return None;
        }
        state.next += 1;
        let id = state.next;
        state.attempts[index] = Some(Attempt::Pending(id));
        Some(id)
    }
    fn finish(&self, browser: Browser, id: u64, result: Result<(), String>) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let attempt = &mut state.attempts[usize::from(browser == Browser::Edge)];
        if matches!(attempt, Some(Attempt::Pending(current)) if *current == id) {
            *attempt = Some(Attempt::Finished(result));
        }
    }
    fn error(&self, browser: Browser) -> Option<String> {
        match &self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attempts[usize::from(browser == Browser::Edge)]
        {
            Some(Attempt::Finished(Err(reason))) => Some(reason.clone()),
            _ => None,
        }
    }
}

fn needed(shared: &Shared, browser: Browser) -> bool {
    matches!(browser, Browser::Chrome | Browser::Edge)
        && shared.chrome_control.config.available()
        && browser.enabled(shared)
        && browser.reconnectable(shared)
        && !super::availability::live(shared, browser)
        && match browser.addon(shared).state {
            AddonState::ChromeClosed => true,
            AddonState::Connecting => shared.chrome_control.owner.starts.pending(browser),
            _ => false,
        }
}

/// Adds the usual card to a closed-browser call in Ask, preserving any prepared personal-read consent.
pub(crate) async fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    tool: &str,
    args: &Value,
    execution: Option<tool_calls::receipts::Execution>,
) -> Result<Option<tool_calls::receipts::Execution>, String> {
    if !(tool == "chrome_session" && args["action"] == "open" || tool == "chrome_execute")
        || super::check::mode(shared, owner).as_deref() != Some("ask")
    {
        return Ok(execution);
    }
    let browser = super::agent_browser::selected(shared, owner, tool, args);
    if !needed(shared, browser) {
        return Ok(execution);
    }
    if tool == "chrome_session" {
        super::gateway::validate_session_args(args)?;
    } else {
        super::gateway::validate_execute_args(args)?;
    }
    // Personal reads already passed their separate approval in prepare_inner.
    if !super::read_consent::personal_read(tool, args) {
        super::check::authorize(shared, owner, tool, args)?;
    }
    if let Err(reason) = super::gateway::validate_session(shared, owner, tool, args) {
        let ended_here = args["sessionId"].as_str().is_some_and(|id| {
            shared
                .chrome_control
                .turn_ended
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(owner)
                .is_some_and(|ids| ids.iter().any(|ended| ended == id))
        });
        if !ended_here {
            return Err(reason);
        }
    }
    super::reconnect::stopped(shared, owner, browser)?;
    tool_calls::approve(shared, owner, source, call, json!({"kind":"approval",
        "effect":{"kind":"tool","toolName":format!("mcp_vw.{tool}"),"input":args},"requiredPermissions":[]})).await?;
    let (shared, owner, tool, args) = (
        Arc::downgrade(shared),
        owner.to_owned(),
        tool.to_owned(),
        args.clone(),
    );
    Ok(Some(Box::pin(async move {
        let shared = shared
            .upgrade()
            .ok_or("Vibe Work closed. Start a new turn before trying again.")?;
        if let Some(reply) = wait(&shared, &owner, browser, true).await? {
            return Ok(reply);
        }
        match execution {
            Some(execution) => execution.await,
            None => super::gateway::call(&shared, &owner, &tool, args).await,
        }
    })))
}

fn active(shared: &Shared, session: &str, browser: Browser, generation: u64) -> Result<(), String> {
    super::reconnect::stopped(shared, session, browser)?;
    if generation != crate::stops::generation(shared, session)
        || shared.store.view(session).is_none()
    {
        return Err(browser.text(
            "Not run: this chat's Chrome work ended. Start a new turn before trying again.",
        ));
    }
    super::check::authorize(
        shared,
        session,
        "chrome_session",
        &json!({"action":"open","profileId":browser.id()}),
    )?;
    Ok(())
}

fn failure(browser: Browser, code: &str, reason: &str) -> Value {
    let recovery = browser.words("Open {fullBrowser}, check that {extension} is on, then try again. Do not start the browser with process or shell tools.")
        .replace("{extension}", vw_vocabulary::label("workNames", "chromeExtension"));
    let instruction = format!("{reason} {recovery}");
    let mut answer = super::owner::own(&format!("Not run: {instruction}"), code).into_value();
    answer["structuredContent"]["addon"] = json!({
        "stateCode":if code == "BROWSER_CONNECT_TIMEOUT" { "connecting" } else { "chromeClosed" },
        "instruction":instruction,
    });
    json!({"vwMcpResult":answer})
}

/// Runs only after the gateway's mode and ownership checks, or the authenticated launch approval.
pub(super) async fn wait(
    shared: &Arc<Shared>,
    session: &str,
    browser: Browser,
    approved: bool,
) -> Result<Option<Value>, String> {
    if !needed(shared, browser) {
        return Ok(None);
    }
    if !approved && super::check::mode(shared, session).as_deref() == Some("ask") {
        return Err(browser.words("Not run: opening {yourBrowser} needs approval in Ask. Call chrome_session with action open and profileId {browser}; Vibe Work shows the permission card before opening it. Do not use process or shell tools."));
    }
    let generation = crate::stops::generation(shared, session);
    active(shared, session, browser, generation)?;
    let deadline = tokio::time::Instant::now() + WAIT;
    if let Some(id) = shared.chrome_control.owner.starts.begin(shared, browser) {
        let weak = Arc::downgrade(shared);
        let chat = session.to_owned();
        tokio::task::spawn_blocking(move || {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let result = (|| {
                active(&shared, &chat, browser, generation)?;
                if super::availability::live(&shared, browser) {
                    return Ok(());
                }
                if !approved && super::check::mode(&shared, &chat).as_deref() == Some("ask") {
                    return Err("Opening the browser now needs approval in Ask".into());
                }
                if !browser.reconnectable(&shared)
                    || !matches!(
                        browser.addon(&shared).state,
                        AddonState::ChromeClosed | AddonState::Connecting
                    )
                {
                    return Err("The browser connection changed before it could be opened".into());
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err("The browser launch did not finish in time".into());
                }
                super::browser::open_paired(&shared, browser)
            })();
            shared
                .chrome_control
                .owner
                .starts
                .finish(browser, id, result);
        });
    }
    loop {
        active(shared, session, browser, generation)?;
        let _ = tokio::time::timeout_at(
            deadline,
            shared.chrome_control.owner.call("chrome_status", json!({})),
        )
        .await;
        active(shared, session, browser, generation)?;
        if super::availability::live(shared, browser) {
            return Ok(None);
        }
        if let Some(reason) = shared.chrome_control.owner.starts.error(browser) {
            return Ok(Some(failure(
                browser,
                "BROWSER_LAUNCH_FAILED",
                &format!("{reason}."),
            )));
        }
        if !needed(shared, browser) {
            return Ok(super::availability::refusal(shared, browser, false));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(Some(failure(browser, "BROWSER_CONNECT_TIMEOUT", &browser.words("Vibe Work asked {fullBrowser} to open, but its extension did not connect within 10 seconds."))));
        }
        tokio::time::sleep_until((tokio::time::Instant::now() + POLL).min(deadline)).await;
    }
}
