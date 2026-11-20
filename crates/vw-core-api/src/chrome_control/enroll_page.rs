//! The pairing page: checking a minted ticket, the page address, and opening the page in the browser.
use super::{
    Arc, Browser, Instant, LockRecover, MINT, OPEN_PAIRING_PAGE, PairPage, Shared, Value, json,
};

/// What the helper answered a mint.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Minted {
    Ticket {
        ticket: String,
        profile: String,
        fingerprint: String,
    },
    NoWaiting,
    /// Ambiguous, malformed, an older helper, or no helper: the plain page.
    Other,
}

pub(super) fn hex64(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn plain_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    (1..=4).contains(&parts.len())
        && parts.iter().enumerate().all(|(i, p)| {
            let max = if i == 0 { 4 } else { 5 };
            !p.is_empty() && p.len() <= max && p.bytes().all(|b| b.is_ascii_digit())
        })
}

pub(super) fn ticket_shape(text: &str) -> bool {
    text.len() == 43
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub(in crate::chrome_control) fn uuid_shape(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub(super) fn minted(
    answer: Option<&Value>,
    expected_fingerprint: Option<&str>,
    browser: Browser,
) -> Minted {
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
pub(in crate::chrome_control) fn page_url(
    extension_id: &str,
    ticket: Option<(&str, &str, &str)>,
) -> String {
    match ticket {
        None => format!("chrome-extension://{extension_id}/ui.html"),
        Some((ticket, profile, fingerprint)) => format!(
            "chrome-extension://{extension_id}/ui.html#vw-enroll={ticket}&p={profile}&f={fingerprint}"
        ),
    }
}

/// The Pair click, with only a fixed expiry marker when requested. A later waiting report never mints for it.
pub(super) fn open_plain(shared: &Shared, caller: &'static str, browser: Browser, page: PairPage) {
    tracing::info!(caller, ?browser, "Chrome enrollment open_plain");
    let chrome = &shared.chrome_control;
    browser.enroll(shared).lock_or_recover().cancel();
    let mut url = page_url(&chrome.config.extension_id, None);
    if let PairPage::Expired = page {
        url.push_str("#vw-enroll-reason=expired");
    }
    if let Err(reason) = browser.open(shared, &url) {
        tracing::warn!(%reason, "Chrome pairing page could not be opened");
    }
}

/// Open pairing page while the browser's extension holds another Vibe Work. The connected extension opens its own
/// page in its own profile's window, in front (`owner.pairingPage.open`); no profile folder is guessed. Without such
/// an extension (none connected, an older extension or helper) the page opens through the browser as before.
pub(super) async fn open_paired_elsewhere(
    shared: &Arc<Shared>,
    caller: &'static str,
    browser: Browser,
) {
    vw_platform::allow_foreground_handover();
    let answer = shared
        .chrome_control
        .owner
        .rpc(OPEN_PAIRING_PAGE, json!({"browser": browser.helper_id()}))
        .await;
    let asked = answer
        .as_ref()
        .and_then(|answer| answer["asked"].as_u64())
        .unwrap_or(0);
    tracing::info!(
        caller,
        ?browser,
        asked,
        "Chrome enrollment open_paired_elsewhere"
    );
    if asked == 0 {
        open_plain(shared, caller, browser, PairPage::Plain);
    } else {
        browser.enroll(shared).lock_or_recover().cancel();
    }
}

/// Asks one mint for `attempt` and opens what it allows. `Ok(false)`: the helper had no waiting connection (the
/// caller arms the retry). The URL and the ticket are never logged: a failed open logs only its fixed reason.
pub(super) async fn mint_and_open(shared: &Arc<Shared>, attempt: &str, browser: Browser) -> bool {
    let chrome = &shared.chrome_control;
    let answer = chrome.owner.rpc(MINT, json!({"attempt": attempt})).await;
    let mut enroll = browser.enroll(shared).lock_or_recover();
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
            super::super::owner::publish(shared);
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
