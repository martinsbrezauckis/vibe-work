//! Asking the person again after Not now or a declined pairing, and the card that says so.
use super::{Arc, Browser, LockRecover, Shared, json};

/// What the person sees after Connect asked again: the question comes with the extension's next reconnect.
pub(in crate::chrome_control) const ASKED_AGAIN: &str = "Vibe Work asks again when {browser} reconnects, within half a minute. To be asked at once, click Check connection in the Vibe Work extension.";

/// The person's Connect for a browser with a "Not now" identity: the helper forgets that answer
/// (`owner.profileUnpair` with `keepPin`: the helper's own saved pin, its current pairing, stays), so the extension's
/// next reconnect asks once more. This includes a paired identity whose key changed. An older helper refuses
/// `keepPin`; for an identity Vibe Work does not hold as paired the plain request then forgets the answer as before,
/// while a paired identity gets no plain request (it would end the pin). Connect also ends the quiet of this
/// browser's removed identities. Returns the hint for the person, or `None` when no such identity waits.
pub(in crate::chrome_control) async fn ask_again(
    shared: &Arc<Shared>,
    origin: crate::Origin,
    browser: Browser,
) -> Option<String> {
    // Only the person's own Connect: a Debug API Connect never undoes their "Not now" or a Remove's quiet.
    if crate::command_origin::debug_api_acts(shared, origin) {
        return None;
    }
    let chrome = &shared.chrome_control;
    chrome.owner.approvals.forget_removed(browser);
    let waiting: Vec<String> = chrome
        .owner
        .approvals
        .declined_for(browser)
        .into_iter()
        .filter(|profile| super::super::profiles::pairing_allowed(chrome, profile))
        .collect();
    if waiting.is_empty() {
        return None;
    }
    for profile in waiting {
        let forgotten = match chrome
            .owner
            .rpc(
                "owner.profileUnpair",
                json!({"profileId":profile,"keepPin":true}),
            )
            .await
        {
            Some(result) => Some(result),
            None if !unpinned(chrome, &profile) => None,
            None => {
                chrome
                    .owner
                    .rpc("owner.profileUnpair", json!({"profileId":profile}))
                    .await
            }
        };
        if forgotten.is_some_and(|result| result["unpaired"] == true) {
            chrome.owner.approvals.forget_declined(&profile);
        }
    }
    super::super::owner::publish(shared);
    Some(browser.words(ASKED_AGAIN))
}

/// An identity Vibe Work does not hold as paired (never paired, or removed): only such an identity may get the plain
/// `owner.profileUnpair`. The helper checks its own pins as well (`keepPin`), so a pin Vibe Work has no record of is
/// not ended by asking again.
pub(super) fn unpinned(chrome: &super::super::ChromeControl, profile: &str) -> bool {
    super::super::profiles::pairing_allowed(chrome, profile)
        && chrome
            .profiles
            .lock_or_recover()
            .records
            .get(&super::super::profiles::canonical(profile))
            .is_none_or(|record| record.removed)
}

/// The browser card while an identity of this browser waits after "Not now": its Connect asks again.
pub(in crate::chrome_control) fn not_now_card(
    chrome: &super::super::ChromeControl,
    view: &mut super::super::AddonView,
    browser: Browser,
) {
    use super::super::AddonState as S;
    let waiting: Vec<String> = chrome
        .owner
        .approvals
        .declined_for(browser)
        .into_iter()
        .filter(|profile| super::super::profiles::pairing_allowed(chrome, profile))
        .collect();
    if !matches!(
        view.state,
        S::Connecting | S::NotConnected | S::ChromeClosed | S::NotPaired | S::Connected
    ) || waiting.is_empty()
    {
        return;
    }
    let removed = waiting
        .iter()
        .any(|profile| chrome.owner.approvals.quiet(profile));
    let a = if browser == Browser::Edge { "an" } else { "a" };
    let a_cap = if browser == Browser::Edge { "An" } else { "A" };
    let waits = if removed {
        browser.words(&format!("The user removed {a} {{browser}} profile in Vibe Work while its extension is still connected in {{browser}}. Ask the user to click Connect {{browser}} in Settings > Browser to pair it again."))
    } else {
        browser.words(&format!("{a_cap} {{browser}} profile asked to connect to Vibe Work and the user chose Not now. Ask the user to click Connect {{browser}} in Settings > Browser to be asked again."))
    };
    if view.state == super::super::AddonState::Connected {
        // Another profile of this browser stays connected: the agent keeps that fact and also learns of the
        // waiting profile. The card's label stays Connected; its step line below tells the person.
        view.instruction = format!("{} {waits}", view.instruction);
    } else {
        // Only a Not now waits for the person's approval; a profile they removed is plainly not connected.
        view.label = browser.words(if removed {
            "Not connected"
        } else {
            "Waiting for your approval"
        });
        view.instruction = waits;
    }
    view.action = Some(super::super::addon::AddonAction::Connect);
    view.step = Some(if removed {
        browser.words(&format!(
            "You removed {a} {{browser}} profile that is still connected in {{browser}}. Click Connect {{browser}} to pair it again."
        ))
    } else {
        browser.words(&format!(
            "{a_cap} {{browser}} profile asked to connect and you chose Not now. Click Connect {{browser}} to be asked again."
        ))
    });
}

/// The identities of this browser that Vibe Work knows (saved or connected), for a whole-browser Remove.
pub(in crate::chrome_control) fn removed_browser(
    chrome: &super::super::ChromeControl,
    browser: Browser,
) {
    let mut known: Vec<String> = chrome
        .profiles
        .lock_or_recover()
        .records
        .keys()
        .filter(|profile| browser.owns_profile(profile))
        .cloned()
        .collect();
    known.extend(
        chrome
            .owner
            .connected_profiles
            .snapshot()
            .into_iter()
            .map(|(profile, _)| profile)
            .filter(|profile| browser.owns_profile(profile)),
    );
    for profile in known {
        chrome.owner.approvals.removed_by_person(&profile);
    }
}
