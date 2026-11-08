//! Automatic Chrome and Edge pairing, app side. A Connect from the app's own window asks the helper for a single-use
//! ticket bound to the helper's current waiting native connection, then opens the extension's own page with it:
//! `chrome-extension://<id>/ui.html#vw-enroll=<ticket>&p=<profileId>&f=<owner fingerprint>`. The updated extension
//! pairs without a click only when its own profile, the proposed owner and the redeemed ticket all match; anything
//! else shows its Pair button, as before.
//!
//! Invariants:
//! - Only a window Connect (`Origin::Ui`) while the Debug API does not drive the window mints. Any other Connect
//!   (the Debug API's own, or the window while the Debug API drives it) drops the current attempt, revokes every
//!   ticket and opens the plain page (the Pair click): it can never reuse an earlier window attempt.
//! - Every window Connect starts a new attempt and revokes older tickets first; stopping the owner revokes too.
//! - An attempt ends on every plain-page path, when the helper connection ends and when the owner stops.
//! - A mint is asked only while the helper's last waiting report carried `enroll: 1` (an updated extension behind a
//!   helper that reports it). With the current Store extension or an older helper the plain page opens.
//! - An attempt lives 10 minutes and retries its mint once, on the next waiting report. A stale or replaced attempt
//!   never mints and never opens its page.
//! - Only expiry of a pending automatic mint adds the fixed `vw-enroll-reason=expired` Pair-page fragment.
//! - The ticket and the URL are never logged, never put in an event and never kept after the page opened.
//! - A browser's Connect consumes only that browser's reports and scoped tickets. The URL carries the raw UUID;
//!   the shipped Chrome-only helper's bare UUID remains accepted for Chrome only.
//!
//! Depends on: the owner pipe (`owner::Owner::rpc`), `browser::open`. Called by: `commands` (Connect), `owner`
//! (waiting reports).
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::Instant;

use super::enroll_browser::Browser;
use crate::core::Shared;

/// Asks the helper for a ticket for its current waiting connection.
pub(super) const MINT: &str = "owner.enroll.mint";
/// Drops every ticket the helper holds.
pub(super) const REVOKE: &str = "owner.enroll.revoke";
/// How long one Connect may still mint (its one retry included).
/// Chrome's first-run screens and Enable can take several minutes. Bound that pending authorization without
/// extending the helper ticket's own lifetime or changing cancellation, browser binding or single use.
pub(super) const ATTEMPT_LIFE: Duration = Duration::from_secs(10 * 60);
/// After the enrollment page opened, the setup says Connecting this long before it falls back to Pair in Chrome.
pub(super) const CONNECTING_FOR: Duration = Duration::from_secs(15);

struct Attempt {
    id: String,
    started: Instant,
    /// The mint still to ask on the next waiting report (the first one when Chrome was not waiting yet, else the
    /// one retry after `no_waiting`).
    mint_on_waiting: bool,
}

/// The manual Pair page, optionally told that its pending automatic mint expired.
#[derive(Clone, Copy)]
enum PairPage {
    Plain,
    Expired,
}

/// The current attempt and what the current helper connection reported.
#[derive(Default)]
pub(crate) struct Enroll {
    attempt: Option<Attempt>,
    /// The last waiting report carried `enroll: 1`.
    waiting_auto: bool,
    /// The helper reported a waiting extension on this connection (not yet paired, no other owner).
    waiting: bool,
    /// This helper connection reports the enrollment fields, so it knows the revoke request.
    helper_knows: bool,
    /// The full owner fingerprint of the last waiting report (64 hex).
    fingerprint: Option<String>,
    /// The installed extension version the last waiting report named (a label only).
    pub(super) extension_version: Option<String>,
    /// When the enrollment page opened for the current attempt.
    opened: Option<Instant>,
}

impl Enroll {
    /// The helper connection ended: its tickets and the current attempt end with it.
    pub(super) fn connection_ended(&mut self, browser: Browser) {
        tracing::info!(
            attempt_dropped = self.attempt.is_some(),
            ?browser,
            "Chrome enrollment connection_ended"
        );
        self.attempt = None;
        self.waiting_auto = false;
        self.waiting = false;
        self.helper_knows = false;
        self.fingerprint = None;
        self.opened = None;
    }

    /// Reads a pairing report's labels (validated and bounded by the helper; they admit nothing here).
    pub(super) fn observe_state(&mut self, state: Option<&str>, params: &Value, browser: Browser) {
        if params.get("enroll").is_some() || params.get("extensionVersion").is_some() {
            self.helper_knows = true;
        }
        if let Some(version) = params["extensionVersion"]
            .as_str()
            .filter(|v| plain_version(v))
        {
            self.extension_version = Some(version.to_owned());
        }
        match state {
            Some("waiting") => {
                self.waiting = true;
                self.waiting_auto = params["enroll"] == 1;
                self.fingerprint = params["fingerprint"]
                    .as_str()
                    .filter(|f| hex64(f))
                    .map(str::to_owned);
            }
            Some("paired" | "mismatch") => {
                self.waiting = false;
                self.waiting_auto = false;
                // Paired: the page did its work; Connecting now comes from the helper's report.
                self.opened = None;
            }
            _ => {}
        }
        tracing::info!(
            state = state_word(state),
            ?browser,
            waiting_auto = self.waiting_auto,
            fingerprint_present = self.fingerprint.is_some(),
            helper_knows = self.helper_knows,
            "Chrome enrollment observe_state"
        );
    }

    /// The helper reported a waiting extension: the pairing step may show.
    pub(super) fn waiting_reported(&self) -> bool {
        self.waiting
    }

    /// A live profile connected: nothing waits any more.
    pub(super) fn connected(&mut self) {
        self.waiting = false;
        self.waiting_auto = false;
        self.opened = None;
    }

    pub(super) fn helper_knows_enroll(&self) -> bool {
        self.helper_knows
    }

    /// Ends the current attempt: nothing mints for it any more.
    fn cancel(&mut self) {
        self.attempt = None;
        self.opened = None;
    }

    /// A new attempt for a window Connect; it replaces any earlier one.
    fn begin(&mut self, now: Instant) -> Option<String> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).ok()?;
        let id = bytes.iter().fold(String::with_capacity(32), |mut id, b| {
            use std::fmt::Write;
            let _ = write!(id, "{b:02x}");
            id
        });
        self.attempt = Some(Attempt {
            id: id.clone(),
            started: now,
            mint_on_waiting: true,
        });
        self.opened = None;
        Some(id)
    }

    fn live(&self, id: &str, now: Instant) -> bool {
        self.attempt
            .as_ref()
            .is_some_and(|a| a.id == id && now.duration_since(a.started) < ATTEMPT_LIFE)
    }

    /// The attempt that may mint now (consumes its pending mint), or the Pair page's fallback reason.
    fn take_mint_on_waiting(&mut self, now: Instant, browser: Browser) -> Result<String, PairPage> {
        tracing::info!(
            waiting = self.waiting,
            ?browser,
            waiting_auto = self.waiting_auto,
            attempt_present = self.attempt.is_some(),
            mint_on_waiting = self.attempt.as_ref().is_some_and(|a| a.mint_on_waiting),
            attempt_age = ?self.attempt.as_ref().map(|a| now.duration_since(a.started)),
            "Chrome enrollment take_mint_on_waiting"
        );
        if !self.waiting_auto {
            return Err(PairPage::Plain);
        }
        let attempt = self.attempt.as_mut().ok_or(PairPage::Plain)?;
        if !attempt.mint_on_waiting {
            return Err(PairPage::Plain);
        }
        if now.duration_since(attempt.started) >= ATTEMPT_LIFE {
            return Err(PairPage::Expired);
        }
        attempt.mint_on_waiting = false;
        Ok(attempt.id.clone())
    }

    /// The setup says Connecting: the enrollment page opened less than 15 s ago.
    pub(super) fn connecting(&self, now: Instant) -> bool {
        self.opened
            .is_some_and(|opened| now.duration_since(opened) < CONNECTING_FOR)
    }
}

/// What the helper answered a mint.
#[derive(Debug, PartialEq, Eq)]
enum Minted {
    Ticket {
        ticket: String,
        profile: String,
        fingerprint: String,
    },
    NoWaiting,
    /// Ambiguous, malformed, an older helper, or no helper: the plain page.
    Other,
}

/// Reduces helper state text to fixed words before tracing it; unknown input is never logged verbatim.
pub(super) fn state_word(state: Option<&str>) -> &'static str {
    match state {
        Some("waiting") => "waiting",
        Some("paired") => "paired",
        Some("mismatch") => "mismatch",
        Some(_) => "unknown",
        None => "missing",
    }
}

fn hex64(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn plain_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    (1..=4).contains(&parts.len())
        && parts.iter().enumerate().all(|(i, p)| {
            let max = if i == 0 { 4 } else { 5 };
            !p.is_empty() && p.len() <= max && p.bytes().all(|b| b.is_ascii_digit())
        })
}

fn ticket_shape(text: &str) -> bool {
    text.len() == 43
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub(super) fn uuid_shape(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn minted(answer: Option<&Value>, expected_fingerprint: Option<&str>, browser: Browser) -> Minted {
    let Some(answer) = answer else {
        tracing::info!(
            kind = "Other",
            reason = "no_answer",
            ?browser,
            "Chrome enrollment mint_and_open"
        );
        return Minted::Other;
    };
    if answer["none"] == "no_waiting" {
        return Minted::NoWaiting;
    }
    let other = |reason: &'static str| {
        // Only the protocol's known words may cross into logs; never format the answer or Minted itself.
        let none_word = match answer["none"].as_str() {
            Some("ambiguous") => "ambiguous",
            Some("no_waiting") => "no_waiting",
            _ if answer.get("none").is_some() => "unknown",
            _ => "absent",
        };
        let reason = if answer.get("none").is_some() {
            "none_word"
        } else {
            reason
        };
        tracing::info!(
            kind = "Other",
            reason,
            none_word,
            ?browser,
            "Chrome enrollment mint_and_open"
        );
        Minted::Other
    };
    let Some(ticket) = answer["ticket"].as_str().filter(|t| ticket_shape(t)) else {
        return other("ticket_shape");
    };
    let Some(profile) = answer["profileId"]
        .as_str()
        .and_then(|p| browser.raw_profile(p))
    else {
        return other("profile_shape");
    };
    let Some(fingerprint) = answer["fingerprint"].as_str().filter(|f| hex64(f)) else {
        return other("fingerprint_shape");
    };
    if expected_fingerprint.is_some_and(|ours| ours != fingerprint) {
        return other("fingerprint_differs");
    }
    Minted::Ticket {
        ticket: ticket.to_owned(),
        profile: profile.to_owned(),
        fingerprint: fingerprint.to_owned(),
    }
}

/// The extension's own page, plain (its Pair button) or carrying a ticket.
pub(super) fn page_url(extension_id: &str, ticket: Option<(&str, &str, &str)>) -> String {
    match ticket {
        None => format!("chrome-extension://{extension_id}/ui.html"),
        Some((ticket, profile, fingerprint)) => format!(
            "chrome-extension://{extension_id}/ui.html#vw-enroll={ticket}&p={profile}&f={fingerprint}"
        ),
    }
}

/// The Pair click, with only a fixed expiry marker when requested. A later waiting report never mints for it.
fn open_plain(shared: &Shared, caller: &'static str, browser: Browser, page: PairPage) {
    tracing::info!(caller, ?browser, "Chrome enrollment open_plain");
    let chrome = &shared.chrome_control;
    browser
        .enroll(shared)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .cancel();
    let mut url = page_url(&chrome.config.extension_id, None);
    if let PairPage::Expired = page {
        url.push_str("#vw-enroll-reason=expired");
    }
    if let Err(reason) = browser.open(shared, &url) {
        tracing::warn!(%reason, "Chrome pairing page could not be opened");
    }
}

/// Asks one mint for `attempt` and opens what it allows. `Ok(false)`: the helper had no waiting connection (the
/// caller arms the retry). The URL and the ticket are never logged: a failed open logs only its fixed reason.
async fn mint_and_open(shared: &Arc<Shared>, attempt: &str, browser: Browser) -> bool {
    let chrome = &shared.chrome_control;
    let answer = chrome.owner.rpc(MINT, json!({"attempt": attempt})).await;
    let mut enroll = browser
        .enroll(shared)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    let live = enroll.live(attempt, now);
    tracing::info!(live, ?browser, "Chrome enrollment mint_and_open");
    if !live {
        // A newer Connect replaced this one, or it expired while the helper answered: nothing opens.
        return true;
    }
    match minted(answer.as_ref(), enroll.fingerprint.as_deref(), browser) {
        Minted::Ticket {
            ticket,
            profile,
            fingerprint,
        } => {
            tracing::info!(
                live,
                kind = "Ticket",
                ?browser,
                "Chrome enrollment mint_and_open"
            );
            enroll.opened = Some(now);
            drop(enroll);
            let url = page_url(
                &chrome.config.extension_id,
                Some((&ticket, &profile, &fingerprint)),
            );
            if let Err(reason) = browser.open(shared, &url) {
                tracing::warn!(%reason, "Chrome pairing page could not be opened");
            }
            super::owner::publish(shared);
            true
        }
        Minted::NoWaiting => {
            tracing::info!(
                live,
                kind = "NoWaiting",
                ?browser,
                "Chrome enrollment mint_and_open"
            );
            false
        }
        Minted::Other => {
            drop(enroll);
            open_plain(shared, "mint_and_open", browser, PairPage::Plain);
            true
        }
    }
}

/// Every Connect: older tickets are revoked. A window Connect (not driven by the Debug API) starts a new attempt
/// and returns it; any other Connect ends the current attempt and returns `None` (it never mints).
pub(super) async fn begin(
    shared: &Arc<Shared>,
    origin: crate::Origin,
    browser: Browser,
) -> Option<String> {
    let chrome = &shared.chrome_control;
    browser
        .requested(shared)
        .store(false, std::sync::atomic::Ordering::SeqCst);
    // The helper's revoke is global: a new browser Connect also ends the other browser's earlier attempt.
    browser
        .other()
        .requested(shared)
        .store(false, std::sync::atomic::Ordering::SeqCst);
    browser
        .other()
        .enroll(shared)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .cancel();
    let (attempt, debug_api_acts) = {
        let mut enroll = browser
            .enroll(shared)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let debug_api_acts = crate::command_origin::debug_api_acts(shared, origin);
        let attempt = if debug_api_acts {
            enroll.cancel();
            None
        } else {
            enroll.begin(Instant::now())
        };
        (attempt, debug_api_acts)
    };
    let reason = match (debug_api_acts, origin, attempt.is_some()) {
        (true, crate::Origin::DebugApi, _) => "debug_api_origin",
        (true, crate::Origin::Ui, _) => "window_drive",
        (false, _, false) => "getrandom_failed",
        (false, _, true) => "started",
    };
    tracing::info!(
        started = attempt.is_some(),
        debug_api_acts,
        ?origin,
        ?browser,
        reason,
        "Chrome enrollment begin"
    );
    // Older helpers answer "method not found", which is fine: they hold no tickets.
    if chrome.owner.is_running() {
        let _ = chrome.owner.rpc(REVOKE, json!({})).await;
    }
    attempt
}

/// Continues Edge setup or opens its Store; the command caller holds the browser lifecycle lock.
pub(super) async fn connect_edge(
    shared: &Arc<Shared>,
    origin: crate::Origin,
) -> Result<(), String> {
    let chrome = &shared.chrome_control;
    if chrome
        .owner
        .credential_missing
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err(vw_vocabulary::texts::CHROME_OWNER_MISSING.into());
    }
    if let Some(reason) = super::edge_policy::refusal(&chrome.config) {
        return Err(reason.to_owned());
    }
    super::refresh_helper_held(chrome).await;
    if let Some(state) = chrome.dir() {
        super::setup::repair_registrations(&chrome.config, state, Some("Microsoft Edge"))?;
    }
    chrome.set_browser_enabled(super::agent_browser::Browser::Edge, true)?;
    super::switches::deliver(shared)
        .await
        .map_err(|error| error.message)?;
    let browser = Browser::Edge;
    let attempt = begin(shared, origin, browser).await;
    browser
        .requested(shared)
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let waiting = browser
        .enroll(shared)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .waiting_reported();
    let other_owner = chrome
        .owner
        .edge_pairing
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .0
        == super::PairingState::OtherOwner;
    let opened = if waiting || other_owner {
        browser
            .requested(shared)
            .store(false, std::sync::atomic::Ordering::SeqCst);
        open_for_waiting(shared, attempt.as_deref(), browser).await;
        Ok(())
    } else {
        super::setup::open_store_for(&chrome.config, true)
    };
    super::start_owner_for(shared, Some("Microsoft Edge")).await;
    super::owner::publish(shared);
    if chrome
        .owner
        .credential_missing
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err(vw_vocabulary::texts::CHROME_OWNER_MISSING.into());
    }
    opened
}

/// Chrome is already waiting for this Connect: mint now when the waiting extension can enroll, else the plain page.
pub(super) async fn open_for_waiting(
    shared: &Arc<Shared>,
    attempt: Option<&str>,
    browser: Browser,
) {
    let can = browser
        .enroll(shared)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .waiting_auto;
    tracing::info!(
        can,
        attempt_present = attempt.is_some(),
        ?browser,
        "Chrome enrollment open_for_waiting"
    );
    let Some(attempt) = attempt.filter(|_| can) else {
        open_plain(shared, "open_for_waiting", browser, PairPage::Plain);
        return;
    };
    {
        let mut enroll = browser
            .enroll(shared)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(a) = enroll.attempt.as_mut().filter(|a| a.id == attempt) {
            // This mint is the first; the one retry stays for the next waiting report.
            a.mint_on_waiting = false;
        }
    }
    if !mint_and_open(shared, attempt, browser).await {
        arm_retry(shared, attempt, browser);
    }
}

fn arm_retry(shared: &Shared, attempt: &str, browser: Browser) {
    let mut enroll = browser
        .enroll(shared)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(a) = enroll.attempt.as_mut().filter(|a| a.id == attempt) {
        a.mint_on_waiting = true;
        browser
            .requested(shared)
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// A waiting (or mismatch) report while the person's Connect waits for Chrome (called by `owner::observe`, inside
/// the pipe reader: the mint runs on its own task). `waiting`: the report is `waiting` (a mismatch never mints).
pub(super) fn on_waiting(shared: &Arc<Shared>, waiting: bool, browser: Browser) {
    let attempt = {
        let mut enroll = browser
            .enroll(shared)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        tracing::info!(
            waiting,
            ?browser,
            waiting_auto = enroll.waiting_auto,
            attempt_present = enroll.attempt.is_some(),
            mint_on_waiting = enroll.attempt.as_ref().is_some_and(|a| a.mint_on_waiting),
            attempt_age = ?enroll.attempt.as_ref().map(|a| now.duration_since(a.started)),
            "Chrome enrollment on_waiting"
        );
        if waiting {
            enroll.take_mint_on_waiting(now, browser)
        } else {
            Err(PairPage::Plain)
        }
    };
    let attempt = match attempt {
        Ok(attempt) => attempt,
        Err(page) => {
            open_plain(shared, "on_waiting", browser, page);
            return;
        }
    };
    let shared = shared.clone();
    tokio::spawn(async move {
        // The retry after this one is used up (`take_mint_on_waiting`): a second no_waiting opens the plain page.
        if !mint_and_open(&shared, &attempt, browser).await {
            open_plain(&shared, "on_waiting_no_waiting", browser, PairPage::Plain);
        }
    });
}

#[cfg(test)]
#[path = "enroll_tests.rs"]
mod tests;
