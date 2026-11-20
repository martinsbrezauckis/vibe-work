//! Whether a Chrome session id belongs to this chat, and the refusal the agent reads when it does not.
use crate::chrome_control::agent_browser::{self, Browser};
use crate::chrome_control::browser_text::Fixed;
use crate::core::Shared;
use serde_json::Value;
use vw_platform::sync::LockRecover;

pub(in crate::chrome_control) fn validate_session(
    shared: &Shared,
    session: &str,
    tool: &str,
    args: &Value,
) -> Result<(), String> {
    if let Some(task) = args["sessionId"].as_str() {
        let owner = shared
            .chrome_control
            .tasks
            .lock_or_recover()
            .get(task)
            .cloned();
        if owner.as_deref() != Some(session) {
            // Ask-mode consent, login and batch preparation return this refusal without `call`'s browser
            // mapping, so it names the session's own browser here.
            let browser = agent_browser::selected(shared, session, tool, args);
            return Err(refusal(shared, session, task, owner.as_deref(), browser));
        }
    } else if tool == "chrome_execute" {
        let browser = agent_browser::selected(shared, session, tool, args);
        return Err(Fixed::OpenSessionFirst.text(browser));
    }
    Ok(())
}

/// A lease or support refusal while the card says nothing usable is connected: the card's diagnosis and the person's
/// step replace it (an old session id after Chrome disconnected says why Chrome is unavailable, not "unknown id").
/// Another chat's session keeps its own refusal ([`Fixed::OtherChat`]): it is a fact about ownership, never about
/// the connection, so it stays the answer while nothing is connected too.
pub(super) fn lease_refusal(
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
    .any(|named| reason == Fixed::OtherChat.text(named))
    {
        return Err(reason);
    }
    crate::chrome_control::availability::refusal(shared, browser, live).ok_or(reason)
}

/// Why `task` is not this chat's live session, in the words the agent reads: this chat's own session that is not
/// retained after a turn says what ended it; a known session of another chat says so; any other id is unknown.
fn refusal(
    shared: &Shared,
    session: &str,
    task: &str,
    owner: Option<&str>,
    browser: Browser,
) -> String {
    if owner.is_some() {
        return Fixed::OtherChat.text(browser);
    }
    let chrome = &shared.chrome_control;
    let shown = {
        let work = chrome.work.lock_or_recover();
        work.chat_of(task).map(|chat| {
            (
                chat == session,
                work.state_of(task, chat).unwrap_or_default().to_owned(),
                work.lowered(task),
            )
        })
    };
    match shown {
        Some((true, state, _)) if state == "disconnected" => Fixed::LostConnection.text(browser),
        Some((true, _, true)) => Fixed::ModeLowered.text(browser),
        Some((true, _, false)) => Fixed::Stopped.text(browser),
        Some((false, _, _)) => Fixed::OtherChat.text(browser),
        // After an app restart an opaque old ID has no browser binding. The browser that
        // happens to be connected or selected in this chat cannot name that missing session.
        None => {
            "Not run: no browser session with this id; open one with chrome_session.".to_owned()
        }
    }
}
