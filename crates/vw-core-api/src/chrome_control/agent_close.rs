//! Agent escape from a retained external browser. Firefox closes its app-owned process through Settings' path;
//! other browsers only receive this chat's normal Stop, then lose this chat's local lease.
use super::{PairingState, agent_browser::Browser, profile_bindings, read_consent::Consent};
use crate::{ask::tool_calls, core::Shared};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use vw_ask::chrome::Tier;
use vw_platform::sync::LockRecover;

const STOP_WAIT: Duration = Duration::from_secs(5);
/// The helper's own-window close reads a final inventory; it gets its own budget per window before the Stop.
const WINDOW_WAIT: Duration = Duration::from_secs(4);
/// The most windows one `close_browser` asks the helper to close (each within `WINDOW_WAIT`).
const MAX_WINDOWS: usize = 6;

fn target(profile: &str) -> Result<Browser, String> {
    match profile {
        "firefox:agent" if cfg!(feature = "your-firefox") => Ok(Browser::Firefox),
        id if id.starts_with("chrome:") && id.len() > "chrome:".len() => Ok(Browser::Chrome),
        id if cfg!(feature = "your-edge") && id.starts_with("edge:") && id.len() > "edge:".len() => Ok(Browser::Edge),
        id if cfg!(feature = "your-safari") && id.starts_with("safari:") && id.len() > "safari:".len() => Ok(Browser::Safari),
        _ => Err("Not run: close_browser needs an exact external profileId from this chat. It cannot close the built-in browser or the user's own Firefox. For the user's browser, stop this chat's session or use tabs.close_owned for its tabs.".into()),
    }
}

/// Ask's exact browser-session approval card. The one-use execution keeps its grant tied to this call.
pub(crate) async fn prepare(
    shared: &Arc<Shared>,
    chat: &str,
    source: &str,
    call_id: &str,
    args: Value,
) -> Result<tool_calls::receipts::Execution, String> {
    // Before any card: a subagent cannot ask the person to close its parent chat's browser (DBK17G-01).
    super::check::refuse_child_source(shared, source, chat, chat, "chrome_session", &args)?;
    super::gateway::validate_session_args(
        &args,
        Browser::from_profile(args["profileId"].as_str().unwrap_or_default()),
    )?;
    let profile = args["profileId"].as_str().unwrap_or_default();
    target(profile)?;
    let consent = Consent(Tier::AcceptEdits, false);
    super::check::authorize_read(shared, chat, "chrome_session", &args, Some(&consent))?;
    super::gateway::halted(
        shared,
        chat,
        Browser::from_profile(profile),
        "chrome_session",
    )?;
    if crate::stops::personally_stopped(shared, chat) {
        return Err(super::person_stop::refusal(Browser::from_profile(profile)));
    }
    let detail = json!({"kind":"approval","vwBrowserTier":Tier::AcceptEdits as u8,
        "effect":{"kind":"tool","toolName":"mcp_vw.chrome_session","input":args},
        "requiredPermissions":[]});
    tool_calls::approve(shared, chat, source, call_id, detail).await?;
    let shared = Arc::downgrade(shared);
    let chat = chat.to_owned();
    Ok(Box::pin(async move {
        let shared = shared
            .upgrade()
            .ok_or(crate::browser_outcomes::SESSION_UNAVAILABLE)?;
        call(&shared, &chat, &args, Some(&consent)).await
    }))
}

pub(super) async fn call(
    shared: &Arc<Shared>,
    chat: &str,
    args: &Value,
    consent: Option<&Consent>,
) -> Result<Value, String> {
    let profile = args["profileId"].as_str().unwrap_or_default();
    let browser = target(profile)?;
    let chrome = &shared.chrome_control;
    // Open takes this lock too. A close cannot race a new session's binding or its Firefox launch.
    let _binding = chrome.binding_changes.lock().await;
    super::gateway::halted(shared, chat, browser, "chrome_session")?;
    if crate::stops::personally_stopped(shared, chat) {
        return Err(super::person_stop::refusal(browser));
    }
    super::check::authorize_read(shared, chat, "chrome_session", args, consent)?;
    let pending = super::person_stop::PendingCall::new(shared, chat, "chrome_session", args);
    pending.finish(shared, chat, browser, &super::owner::own("", "OK"))?;
    if crate::stops::personally_stopped(shared, chat) {
        return Err(super::person_stop::refusal(browser));
    }
    if browser == Browser::Firefox {
        return close_firefox(shared, chat, profile, &pending).await;
    }
    close_person_browser(shared, chat, profile, browser, &pending, consent).await
}

async fn close_firefox(
    shared: &Arc<Shared>,
    chat: &str,
    profile: &str,
    pending: &super::person_stop::PendingCall,
) -> Result<Value, String> {
    let chrome = &shared.chrome_control;
    if chrome.work.lock_or_recover().other_live_firefox(chat)
        || profile_bindings::other_firefox_hold(chrome, chat)
    {
        return Err("Not run: another chat is using the agent's Firefox. Wait for that chat to finish or ask the user.".into());
    }
    let state = chrome.owner.firefox_pairing.lock_or_recover().0;
    let already = matches!(
        state,
        PairingState::WindowClosed | PairingState::NotInstalled
    );
    if !already {
        super::firefox_commands::window(shared, false)
            .await
            .map_err(|error| error.to_string())?;
    }
    pending.finish(shared, chat, Browser::Firefox, &super::owner::own("", "OK"))?;
    if crate::stops::personally_stopped(shared, chat) {
        return Err(super::person_stop::refusal(Browser::Firefox));
    }
    release(shared, chat, profile);
    let name = vw_vocabulary::texts::FIREFOX_NAME.trim_start_matches("Your ");
    Ok(reply(&if already {
        format!("The agent's {name} is already closed. Open a new browser session when needed.")
    } else {
        format!("The agent's {name} was closed. Open a new browser session when needed.")
    }))
}

async fn close_person_browser(
    shared: &Arc<Shared>,
    chat: &str,
    requested: &str,
    browser: Browser,
    pending: &super::person_stop::PendingCall,
    consent: Option<&Consent>,
) -> Result<Value, String> {
    let chrome = &shared.chrome_control;
    let initial_status = fetch_status(shared).await;
    let resolved = resolve_from_status(chrome, initial_status.as_ref(), requested)
        .map_err(|count| super::profile_ids::ambiguous_profile_refusal(browser, count))?;
    let profile = resolved.as_str();
    let known = profile_bindings::close_known(chrome, chat, profile)
        || chrome.work.lock_or_recover().has_profile(chat, profile);
    if !known {
        return Err(format!(
            "Not run: this chat has no {} browser session for profileId {requested}. Open a session first.",
            browser.name()
        ));
    }
    let ids = close_candidates(chrome, chat, profile);
    let uncertain = profile_bindings::uncertain(chrome, chat);
    let mut attempted = false;
    let mut closed_live = false;
    let mut confirmed = true;
    let mut windows = Closing::default();
    let mut resolved_profiles = Vec::new();
    for id in &ids {
        // A disconnected row is already ended; a stale lease alone still gets a best-effort Stop.
        if matches!(
            chrome.work.lock_or_recover().state_of(id, chat),
            Some("stopped" | "disconnected")
        ) {
            continue;
        }
        attempted = true;
        let cached = (!closed_live).then_some(initial_status.as_ref()).flatten();
        closed_live = true;
        let resolved =
            close_own_windows(shared, chat, profile, id, consent, cached, &mut windows).await;
        if !resolved_profiles.contains(&resolved) {
            resolved_profiles.push(resolved);
        }
        confirmed &= stop_session(chrome, id).await;
    }
    if !attempted && uncertain {
        // A failed open may have no session ID. The existing chat-scoped delete control is the
        // only helper release request available until that uncertain session identifies itself.
        // `closed_live` stays false: `close_own_windows` never ran, so any dropped-window mark stays.
        attempted = true;
        confirmed = false;
        let _ = tokio::time::timeout(
            STOP_WAIT,
            chrome.owner.call(
                "chrome_session",
                json!({"action":"delete","profileId":profile,"_clientSessionId":chat}),
            ),
        )
        .await;
    }
    pending.finish(shared, chat, browser, &super::owner::own("", "OK"))?;
    if crate::stops::personally_stopped(shared, chat) {
        return Err(super::person_stop::refusal(browser));
    }
    release(shared, chat, profile);
    let name = browser.name();
    let (earlier, earlier_warned, unresolvable, has_canonical) = forget_windows(
        shared,
        chat,
        profile,
        &resolved_profiles,
        &windows,
        closed_live,
        confirmed,
    );
    let clear_unclosed = !has_canonical
        && ((closed_live && confirmed && !earlier && !windows.live_failed()) || unresolvable);
    profile_bindings::sync_window_marks(chrome, chat, profile, clear_unclosed, unresolvable);
    let attempt = if !attempted {
        CloseAttempt::Ended
    } else if confirmed {
        CloseAttempt::StoppedConfirmed
    } else {
        CloseAttempt::StoppedUnconfirmed
    };
    Ok(reply(&answer_text(
        name,
        attempt,
        earlier,
        earlier_warned,
        &windows,
    )))
}

fn close_candidates(chrome: &super::ChromeControl, chat: &str, profile: &str) -> Vec<String> {
    let mut ids = profile_bindings::close_candidates(chrome, chat, profile);
    ids.extend(
        chrome
            .work
            .lock_or_recover()
            .close_candidates(chat, profile),
    );
    ids.sort();
    ids.dedup();
    ids
}

async fn stop_session(chrome: &super::ChromeControl, id: &str) -> bool {
    let answer = tokio::time::timeout(
        STOP_WAIT,
        chrome
            .owner
            .call("chrome_session", json!({"action":"stop","sessionId":id})),
    )
    .await;
    answer.is_ok_and(|answer| {
        let body = &answer.value()["structuredContent"];
        answer.value()["isError"] != true
            && body["error"].is_null()
            && matches!(body["state"].as_str(), Some("stopped" | "disconnected"))
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseAttempt {
    Ended,
    StoppedConfirmed,
    StoppedUnconfirmed,
}

/// The agent-facing answer of one `close_browser`.
fn answer_text(
    name: &str,
    attempt: CloseAttempt,
    earlier: bool,
    earlier_warned: bool,
    windows: &Closing,
) -> String {
    let earlier_clause = if earlier_warned {
        "; an agent window reported earlier may still be open; tell the user"
    } else {
        ""
    };
    match attempt {
        CloseAttempt::Ended if earlier => format!(
            "The browser session of this chat in {name} ended by close_browser. An agent window from before the browser lost its connection may still be open; to close it, open a new browser session and use close_browser again, or tell the user. The chat may open another browser."
        ),
        CloseAttempt::Ended => format!(
            "The browser session of this chat in {name} ended by close_browser{earlier_clause}. The chat may open another browser."
        ),
        CloseAttempt::StoppedConfirmed if windows.failed() && windows.closed => format!(
            "The browser session of this chat in {name} was stopped by close_browser. Some of the agent's own tabs could not be closed (a tab the user moved to another window, a window from before the browser reconnected or restarted, or a close the browser did not confirm); tell the user they may still be open. The chat may open another browser."
        ),
        CloseAttempt::StoppedConfirmed if windows.failed() => format!(
            "The browser session of this chat in {name} was stopped by close_browser. The agent's own tabs could not be closed; tell the user they may still be open. The chat may open another browser."
        ),
        CloseAttempt::StoppedConfirmed if windows.closed => format!(
            "The browser session of this chat in {name} was stopped by close_browser and the agent's own tabs in this browser session were closed; tabs the user opened or changed stay open, and so does a window that still holds one{earlier_clause}. The chat may open another browser."
        ),
        CloseAttempt::StoppedConfirmed => format!(
            "The browser session of this chat in {name} was stopped by close_browser{earlier_clause}. The chat may open another browser."
        ),
        CloseAttempt::StoppedUnconfirmed => format!(
            "The browser did not confirm Stop. The agent's tabs may still be open in {name}; tell the user. The session of this chat ended by close_browser and the chat may open another browser."
        ),
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum LiveFailure {
    #[default]
    None,
    /// A live check failed on a window still tracked for retry in `agent_windows`.
    Tracked,
    /// A live check left unclosed tabs not tracked in `agent_windows` (reload, moved tab, cap).
    Unresolvable,
}

/// What `close_browser`'s window closes did across this chat's live sessions.
#[derive(Default)]
struct Closing {
    /// The helper confirmed at least one window close.
    closed: bool,
    /// True when `nominate` saw windows dropped on a browser incarnation change.
    dropped: bool,
    /// Live close/inventory failure status across the chat's sessions.
    live_failure: LiveFailure,
    /// Windows whose `window.close` call was not confirmed; checked against remaining `agent_windows`.
    failed_windows: Vec<u64>,
    /// Windows already asked; each at most once per `close_browser`.
    tried: Vec<u64>,
    /// Windows whose close the helper confirmed. A tried window absent here (refused, timed out, outcome unknown)
    /// stays remembered, so a later `close_browser` can nominate it again (DBK17R-02).
    confirmed: Vec<u64>,
    /// Remembered windows a complete inventory showed without this chat's tabs, or not at all.
    settled: Vec<u64>,
}

impl Closing {
    fn failed(&self) -> bool {
        self.dropped || self.live_failed()
    }
    fn live_failed(&self) -> bool {
        self.live_failure != LiveFailure::None
    }
    fn untracked_failure(&self) -> bool {
        self.live_failure == LiveFailure::Unresolvable
    }
    fn record_live_failure(&mut self, failed: bool) {
        if failed && self.live_failure == LiveFailure::None {
            self.live_failure = LiveFailure::Tracked;
        }
    }
    fn record_unresolvable(&mut self, unresolvable: bool) {
        if unresolvable {
            self.live_failure = LiveFailure::Unresolvable;
        }
    }
}

/// One window close's complete inventory: windows holding this chat's own tabs, and every window seen.
struct Inventory {
    owned: Vec<u64>,
    present: Vec<u64>,
}

/// The agent's own windows (owner 2026-11-23: `close_browser` closes the agent's own windows in every browser; owner
/// 2026-11-23 evening after the native disconnect runs: also the windows of the chat's earlier sessions). After any lost
/// connection the extension hands the agent's windows to the person and the next session opens a new window, so a chat
/// can hold several. Candidates are ONLY windows the agent itself opened: this session's own window (the helper's
/// session `windowId`) and the windows this chat's agent created tabs in (`remember`; `tabs.create` always opens in the
/// agent's own window). A window the inventory lists with this chat's tab is never a candidate by that alone: the person
/// may have moved an agent-created tab into their own window and handed it over (DBK17R-01); such a tab is reported,
/// never closed (owner: the person's windows and signed-in tabs are not the agent's to close). The helper's
/// `window.close` closes only this chat's exact original-window tabs in a window and refuses another chat's window.
/// Same gateway path and mode checks as the agent's own call.
async fn close_own_windows(
    shared: &Arc<Shared>,
    chat: &str,
    profile: &str,
    id: &str,
    consent: Option<&Consent>,
    cached_status: Option<&Value>,
    out: &mut Closing,
) -> String {
    let live = live_status(shared, id, profile, cached_status).await;
    let current = live.window;
    // DBK17S-01: a remembered window is nominated only while the helper reports the browser incarnation it was opened
    // in. After a browser restart Chrome can reuse the number for a person's window, and the helper would close a tab
    // the person moved there. Any other window is dropped unasked and reported: its tabs may still be open.
    let (remembered, dropped) = shared
        .chrome_control
        .agent_windows
        .lock_or_recover()
        .nominate(chat, &live.profile, live.incarnation.as_deref());
    out.dropped |= dropped;
    let mut pending: std::collections::VecDeque<u64> = current
        .into_iter()
        .chain(remembered.iter().copied())
        .collect();
    let mut known: Option<Inventory> = None;
    while let Some(window) = pending.pop_front() {
        if out.tried.contains(&window) || out.settled.contains(&window) {
            continue;
        }
        if let Some(inventory) = &known
            && !inventory.owned.contains(&window)
        {
            // An agent window without this chat's tabs: closed already, or no longer the chat's to close.
            out.record_unresolvable(inventory.present.contains(&window));
            out.settled.push(window);
            continue;
        }
        if out.tried.len() >= MAX_WINDOWS {
            out.record_live_failure(true);
            out.record_unresolvable(Some(window) == current && !remembered.contains(&window));
            break;
        }
        out.tried.push(window);
        match close_window(shared, chat, id, window, consent).await {
            None => {
                out.record_live_failure(true);
                out.failed_windows.push(window);
            }
            Some(inventory) => {
                out.closed = true;
                out.confirmed.push(window);
                if let Some(inventory) = inventory {
                    // An agent-created tab in a window the agent did not open stays open and is reported.
                    let moved = inventory.owned.iter().any(|window| {
                        Some(*window) != current
                            && !remembered.contains(window)
                            && !out.tried.contains(window)
                    });
                    out.record_unresolvable(moved);
                    known = Some(inventory);
                }
            }
        }
    }
    live.profile
}

/// Resolves `profile` to the helper's exact `profileId` when `live_profiles` has no exact match and names one
/// matching canonical profile (for example, `chrome_status.selectedProfile` lowercases an uppercase Chrome UUID).
/// Distinct exact helper IDs keep their exact spellings; ambiguous lists return `Err(count)` and never guess.
fn resolve_live_profile<'a>(
    live_profiles: impl IntoIterator<Item = &'a str>,
    profile: &str,
) -> Result<String, usize> {
    let wanted = super::profiles::canonical(profile);
    let mut matches = Vec::new();
    for candidate in live_profiles {
        if candidate == profile {
            return Ok(profile.to_owned());
        }
        if target(candidate).is_ok()
            && super::profiles::canonical(candidate) == wanted
            && !matches.contains(&candidate)
        {
            matches.push(candidate);
        }
    }
    match matches.as_slice() {
        [only] => Ok((*only).to_owned()),
        many if many.len() > 1 => Err(many.len()),
        _ => Ok(profile.to_owned()),
    }
}

async fn fetch_status(shared: &Arc<Shared>) -> Option<Value> {
    tokio::time::timeout(
        WINDOW_WAIT,
        shared.chrome_control.owner.call("chrome_status", json!({})),
    )
    .await
    .ok()
    .map(super::owner::Answer::into_value)
}

fn resolve_from_status(
    chrome: &super::ChromeControl,
    status: Option<&Value>,
    profile: &str,
) -> Result<String, usize> {
    if let Some(rows) = status
        .and_then(|value| value["structuredContent"]["profiles"].as_array())
        .filter(|rows| !rows.is_empty())
    {
        return resolve_live_profile(
            rows.iter().filter_map(|row| row["profileId"].as_str()),
            profile,
        );
    }
    let live = chrome.owner.connected_profiles.snapshot();
    resolve_live_profile(live.iter().map(|(id, _)| id.as_str()), profile)
}

/// What the helper's status says about one live session: its own window (if it has one), the browser incarnation
/// (`browserSessionId`) of its profile, and the resolved helper profile ID. Both window and incarnation stay `None`
/// when the status did not answer in time.
#[derive(Default)]
struct Live {
    window: Option<u64>,
    incarnation: Option<String>,
    profile: String,
}

async fn live_status(
    shared: &Arc<Shared>,
    id: &str,
    profile: &str,
    cached_status: Option<&Value>,
) -> Live {
    let fetched;
    let status = if let Some(value) = cached_status {
        value
    } else {
        let Some(value) = fetch_status(shared).await else {
            let resolved = resolve_from_status(&shared.chrome_control, None, profile)
                .unwrap_or_else(|_| profile.to_owned());
            return Live {
                window: None,
                incarnation: None,
                profile: resolved,
            };
        };
        fetched = value;
        &fetched
    };
    let data = &status["structuredContent"];
    let resolved = resolve_from_status(&shared.chrome_control, Some(status), profile)
        .unwrap_or_else(|_| profile.to_owned());
    let session = data["sessions"].as_array().and_then(|rows| {
        rows.iter().find(|row| {
            row["sessionId"] == id && row["profileId"].as_str() == Some(resolved.as_str())
        })
    });
    let row = data["profiles"].as_array().and_then(|rows| {
        rows.iter()
            .find(|row| row["profileId"].as_str() == Some(resolved.as_str()))
    });
    Live {
        window: session.and_then(|row| row["windowId"].as_u64()),
        incarnation: row
            .and_then(|row| row["browserSessionId"].as_str())
            .filter(|incarnation| !incarnation.is_empty())
            .map(str::to_owned),
        profile: resolved,
    }
}

/// One helper `window.close`. `None`: not confirmed (refused, timed out, or outcome unknown); `Some(inventory)`: the
/// helper confirmed that this chat's own tabs there closed, with its final inventory when that is complete.
async fn close_window(
    shared: &Arc<Shared>,
    chat: &str,
    id: &str,
    window: u64,
    consent: Option<&Consent>,
) -> Option<Option<Inventory>> {
    // close_browser itself runs inside the gateway; boxing this nested gateway call ends the future type's recursion
    // (without it the app crate's `Send` check overflows the trait solver: E0275 in `tauri::generate_handler`).
    let call: std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, String>> + Send + '_>,
    > = Box::pin(super::gateway::call_one_inner(
        shared,
        chat,
        "chrome_execute",
        json!({"sessionId":id,"action":"window.close","args":{"windowId":window}}),
        consent,
        None,
    ));
    let answer = tokio::time::timeout(WINDOW_WAIT, call).await.ok()?.ok()?;
    let result = &answer["vwMcpResult"];
    let body = &result["structuredContent"];
    let page = &body["untrustedPageContent"];
    let data = if body["data"].is_object() {
        &body["data"]
    } else {
        &page["data"]
    };
    let confirmed = result["isError"] != true
        && body["error"].is_null()
        && page["error"].is_null()
        && (data["windowClosed"] == true || data["allOwnedTabsClosed"] == true);
    if !confirmed {
        return None;
    }
    let inventory = &data["inventory"];
    if inventory["complete"] != true {
        return Some(None);
    }
    let (mut owned, mut present) = (Vec::new(), Vec::new());
    for tab in inventory["tabs"].as_array().into_iter().flatten() {
        let Some(window) = tab["windowId"].as_u64() else {
            continue;
        };
        if !present.contains(&window) {
            present.push(window);
        }
        if tab["owned"] == true && !owned.contains(&window) {
            owned.push(window);
        }
    }
    Some(Some(Inventory { owned, present }))
}

/// Records the window of a tab this chat's agent created in an external Chrome, Edge or Safari profile.
pub(super) fn remember(shared: &Shared, chat: &str, tool: &str, args: &Value, result: &Value) {
    if tool != "chrome_execute" || args["action"] != "tabs.create" || result["isError"] == true {
        return;
    }
    let body = &result["structuredContent"];
    let Some(window) = [&body["data"], &body["untrustedPageContent"]["data"]]
        .into_iter()
        .find_map(|data| data["tab"]["windowId"].as_u64())
    else {
        return;
    };
    let Some(profile) = super::receipt::profile(shared, tool, args).filter(|profile| {
        matches!(
            target(profile),
            Ok(Browser::Chrome | Browser::Edge | Browser::Safari)
        )
    }) else {
        return;
    };
    let live = shared.chrome_control.owner.connected_profiles.snapshot();
    let profile =
        resolve_live_profile(live.iter().map(|(id, _)| id.as_str()), &profile).unwrap_or(profile);
    // Bound to the browser incarnation the app last saw for the profile; with none known nothing is remembered.
    let added = shared
        .chrome_control
        .agent_windows
        .lock_or_recover()
        .add(chat, &profile, window);
    if added {
        profile_bindings::mark_unclosed_window(&shared.chrome_control, chat, &profile);
    }
}

/// This chat's remembered agent windows in this profile, newest first (tests read what is remembered).
#[cfg(all(test, feature = "your-firefox", feature = "your-edge"))]
pub(super) fn remembered(shared: &Shared, chat: &str, profile: &str) -> Vec<u64> {
    shared
        .chrome_control
        .agent_windows
        .lock_or_recover()
        .of(chat, profile)
}

/// Forgets the windows whose close the helper confirmed and those a complete inventory settled, and clears any
/// observer-dropped mark once a live close ran `close_own_windows` (`closed_live`). Returns `(earlier,
/// had_warned_before, unresolvable_now, has_canonical)`: `unresolvable_now` marks `(chat, profile)` so later closes
/// keep reporting that an earlier window may still be open until the chat is deleted.
fn forget_windows(
    shared: &Shared,
    chat: &str,
    profile: &str,
    resolved_profiles: &[String],
    windows: &Closing,
    closed_live: bool,
    confirmed: bool,
) -> (bool, bool, bool, bool) {
    let mut all = shared.chrome_control.agent_windows.lock_or_recover();
    let had_warned_before = all.has_warned(chat, profile);
    let had_dropped_before = all.has_dropped(chat, profile);
    let targets: Vec<&str> = if resolved_profiles.is_empty() {
        vec![profile]
    } else {
        resolved_profiles.iter().map(String::as_str).collect()
    };
    for target in &targets {
        all.forget(chat, target, |window| {
            windows.confirmed.contains(&window) || windows.settled.contains(&window)
        });
    }
    if closed_live {
        all.clear_dropped(chat, profile);
    }
    let remaining_for_targets: Vec<u64> = targets
        .iter()
        .flat_map(|target| all.of(chat, target))
        .collect();
    let earlier = !remaining_for_targets.is_empty() || all.has_dropped(chat, profile);
    let untracked_failed = windows
        .failed_windows
        .iter()
        .any(|window| !remaining_for_targets.contains(window));
    let consumed_dropped = closed_live && (windows.dropped || had_dropped_before);
    let unresolvable =
        consumed_dropped || windows.untracked_failure() || untracked_failed || !confirmed;
    if unresolvable {
        all.mark_warned(chat, profile);
    }
    let has_canonical = all.has_canonical(chat, profile);
    (earlier, had_warned_before, unresolvable, has_canonical)
}

fn release(shared: &Shared, chat: &str, profile: &str) {
    let chrome = &shared.chrome_control;
    let mut ids = chrome
        .work
        .lock_or_recover()
        .close_chat_profile(chat, profile);
    ids.extend(profile_bindings::close_agent(chrome, chat, profile));
    ids.sort();
    ids.dedup();
    chrome
        .tasks
        .lock_or_recover()
        .retain(|id, owner| owner != chat || !ids.contains(id));
    chrome.output_cache.lock_or_recover().clear_chat(chat);
    super::frames::sync_chat(shared, chat);
    super::owner::publish(shared);
}

fn reply(message: &str) -> Value {
    json!({"vwMcpResult":{"content":[{"type":"text","text":message}],
        "structuredContent":{"status":"ended","message":message},"isError":false}})
}

/// App-owned recovery guidance beside, never inside, the helper's untrusted answer.
pub(super) fn hint(shared: &Shared, tool: &str, args: &Value, result: &mut Value) {
    let body = &result["structuredContent"];
    let payload = &body["untrustedPageContent"];
    let code = payload["error"]["code"]
        .as_str()
        .or_else(|| payload["hostError"]["code"].as_str())
        .or_else(|| body["error"]["code"].as_str());
    let message = payload["error"]["message"]
        .as_str()
        .or_else(|| payload["hostError"]["message"].as_str())
        .unwrap_or_default();
    let stopped =
        code == Some("STOP_UNCONFIRMED") || payload["suspensionReason"] == "STOP_UNCONFIRMED";
    let unanswered_window = code == Some("OUTCOME_UNKNOWN")
        && message.contains("did not answer the new-window request");
    if !stopped && !unanswered_window {
        return;
    }
    let Some(profile) = super::receipt::profile(shared, tool, args) else {
        return;
    };
    let hint = if profile == "firefox:agent" {
        "Stop this session, then use chrome_session close_browser (profileId firefox:agent) and open a new browser session.".to_owned()
    } else if stopped && target(&profile).is_ok() {
        format!(
            "Use chrome_session close_browser (profileId {profile}) to end this chat's hold, then open a new browser session."
        )
    } else {
        return;
    };
    result["structuredContent"]["recovery"] = json!(hint);
    super::output::sync_text(result);
}
