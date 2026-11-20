//! Lease checks around a Chrome tool call: the authorization recheck after every wait, and the admission
//! of an answer to this chat's session lease (new session ids, `chrome_status` rows scoped to the chat).
use crate::chrome_control::{agent_browser::Browser, owner};
use crate::core::Shared;
use serde_json::{Value, json};
use vw_platform::sync::LockRecover;

/// The person's Stop of this browser in this turn ends any acting call; an open browser dialog also holds
/// `chrome_execute`. One check for every point where a call starts or resumes (see `gateway::call_entry`).
pub(in crate::chrome_control) fn halted(
    shared: &Shared,
    session: &str,
    browser: Browser,
    tool: &str,
) -> Result<(), String> {
    crate::chrome_control::reconnect::stopped(shared, session, browser)?;
    if tool == "chrome_execute" {
        shared.browser_dialogs.check()?;
    }
    Ok(())
}

/// Waiting for the adapter or a dialog never preserves an old browser-action authorization: the person's Stop, the
/// chat mode and (`chrome_execute`) an open dialog are read again, in that order of refusal.
pub(in crate::chrome_control) fn recheck(
    shared: &Shared,
    session: &str,
    browser: Browser,
    tool: &str,
    args: &Value,
    consent: Option<&crate::chrome_control::read_consent::Consent>,
) -> Result<(), String> {
    crate::chrome_control::reconnect::stopped(shared, session, browser)?;
    crate::chrome_control::check::authorize_read(shared, session, tool, args, consent)?;
    if tool == "chrome_execute" {
        shared.browser_dialogs.check()?;
    }
    Ok(())
}

pub(in crate::chrome_control) fn scope_result(
    shared: &Shared,
    session: &str,
    tool: &str,
    request: bool,
    browser: Browser,
    pending: &crate::chrome_control::person_stop::PendingCall,
    answer: &mut owner::Answer,
) -> Result<(), String> {
    let mut tasks = shared.chrome_control.tasks.lock_or_recover();
    let mut used_instead = None;
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
        let profile = result["structuredContent"]["profileId"].as_str().ok_or("The browser did not identify the opened profile. Reconnect the profile before opening a browser session.")?;
        let work = shared.chrome_control.work.lock_or_recover();
        if work
            .profile_of(id)
            .is_some_and(|old| !crate::chrome_control::profile_ids::same(old, profile))
            || work.chat_of(id).is_some_and(|old| old != session)
        {
            return Err("This browser session is already associated with another chat or profile. Reconnect this profile before opening a new browser session.".into());
        }
        drop(work);
        let picked =
            (!crate::chrome_control::profile_bindings::locked(&shared.chrome_control, session))
                .then(|| {
                    shared
                        .chrome_control
                        .picked_profiles
                        .lock_or_recover()
                        .get(session)
                        .cloned()
                })
                .flatten();
        crate::chrome_control::profile_bindings::remember(
            &shared.chrome_control,
            session,
            profile,
            Some(id),
        )?;
        let work = shared.chrome_control.work.lock_or_recover();
        tasks.retain(|id, owner| {
            owner != session
                || work
                    .profile_of(id)
                    .is_some_and(|profile| Browser::from_profile(profile) != browser)
        });
        drop(work);
        tasks.insert(id.to_owned(), session.to_owned());
        shared.chrome_control.work.lock_or_recover().open(
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
        used_instead = picked
            .filter(|picked| !crate::chrome_control::profile_ids::same(picked, profile))
            .map(|picked| (picked, profile.to_owned()));
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
    if let Some((picked, used)) = used_instead {
        notice_used_instead(shared, session, &picked, &used);
    }
    if request {
        owner::publish(shared);
    }
    Ok(())
}

fn notice_used_instead(shared: &Shared, session: &str, picked: &str, used: &str) {
    shared
        .chrome_control
        .picked_profiles
        .lock_or_recover()
        .remove(session);
    let message = vw_vocabulary::texts::BROWSER_PROFILE_USED_INSTEAD
        .replace(
            "{picked}",
            &crate::chrome_control::profiles::label(&shared.chrome_control, picked),
        )
        .replace(
            "{used}",
            &crate::chrome_control::profiles::label(&shared.chrome_control, used),
        );
    let _ = shared.store.apply(vw_session::Notification::new(
        "warning",
        json!({"sessionId":session,"warning":{"code":"browser_profile_used_instead","message":message}}),
    ));
}
