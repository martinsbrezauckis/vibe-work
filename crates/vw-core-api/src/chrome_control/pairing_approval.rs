//! Native person confirmation for an exact helper pairing request. Depends on the owner pipe and ask service;
//! called only by trusted owner notifications, with cancellation on helper loss or browser disable.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tokio::task::AbortHandle;
use vw_gate::{Confirmation, ConfirmationRequest, Requester};

use super::{agent_browser::Browser, enroll, profile_presence::Found};
use crate::core::Shared;

/// The person-only native confirmation shared by all connected browsers. It has no public approval command.
pub const BROWSER_PAIRING_DIALOG_METHOD: &str = "chrome/control/pair";
const LIFE: Duration = Duration::from_secs(10 * 60);
/// The dialog waits at most this long for the asking profile's name; without it, it names the browser only.
const NAME_WAIT: Duration = Duration::from_secs(2);

struct Pending {
    profile: String,
    opened: Instant,
    task: AbortHandle,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Open requests by request id, the Chrome/Edge identities whose last request was answered "Not now", and the
/// Chrome/Edge identities the person removed in Settings during this app run.
/// The helper keeps a "Not now" for its request's life, so the extension's automatic reconnects do not ask again;
/// only the person's Connect in Settings (`ask_again`) lets the next reconnect ask. A removed identity's requests are
/// answered "Not now" without a dialog until that Connect or a pairing; this survives helper restarts.
#[derive(Default)]
pub(super) struct Approvals(
    Mutex<BTreeMap<String, Pending>>,
    Mutex<BTreeSet<String>>,
    Mutex<BTreeSet<String>>,
);

impl Approvals {
    pub(super) fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.declined().clear();
    }

    fn declined(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.1.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn removed(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.2.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The person removed this Chrome/Edge identity: its next requests do not open the dialog.
    pub(super) fn removed_by_person(&self, profile: &str) {
        if matches!(
            Browser::from_profile(profile),
            Browser::Chrome | Browser::Edge
        ) {
            self.removed().insert(profile.to_owned());
        }
    }

    /// The removal could not be completed: the identity asks as before.
    pub(super) fn removal_failed(&self, profile: &str) {
        self.removed()
            .retain(|other| !super::profile_ids::same(other, profile));
    }

    /// Connect, or a whole-browser removal that could not be completed: this browser's identities ask as before.
    pub(super) fn forget_removed(&self, browser: Browser) {
        self.removed()
            .retain(|profile| !browser.owns_profile(profile));
    }

    fn quiet(&self, profile: &str) -> bool {
        self.removed()
            .iter()
            .any(|other| super::profile_ids::same(other, profile))
    }

    /// A new request, a pairing or Connect replaces this identity's "Not now".
    fn forget_declined(&self, profile: &str) {
        self.declined()
            .retain(|other| !super::profile_ids::same(other, profile));
    }

    /// The identities of this browser that wait after the person's "Not now".
    pub(super) fn declined_for(&self, browser: Browser) -> Vec<String> {
        self.declined()
            .iter()
            .filter(|profile| browser.owns_profile(profile))
            .cloned()
            .collect()
    }

    pub(super) fn cancel_browser(&self, browser: Browser) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, pending| !browser.owns_profile(&pending.profile));
        self.declined()
            .retain(|profile| !browser.owns_profile(profile));
    }

    /// A native pairing approval still waits for this identity (an expired one no longer counts).
    pub(super) fn waiting(&self, profile: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .any(|pending| {
                pending.opened.elapsed() < LIFE
                    && super::profile_ids::same(&pending.profile, profile)
            })
    }

    pub(super) fn cancel_profile(&self, profile: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, pending| !super::profile_ids::same(&pending.profile, profile));
        self.forget_declined(profile);
    }

    pub(super) fn paired(&self, profile: &str) {
        self.forget_declined(profile);
        self.removal_failed(profile);
        for pending in self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            if pending.profile == profile {
                pending.task.abort();
            }
        }
    }
}

pub(super) fn observe(shared: &Arc<Shared>, params: &Value) {
    let (Some(id), Some(profile)) = (params["requestId"].as_str(), params["profileId"].as_str())
    else {
        return;
    };
    let browser = Browser::from_profile(profile);
    let raw = profile
        .strip_prefix(&format!("{}:", browser.id()))
        .unwrap_or(profile);
    if !enroll::uuid_shape(id)
        || !enroll::uuid_shape(raw)
        || params.as_object().is_none_or(|p| p.len() != 2)
        || !browser.enabled(shared)
        || !super::profiles::pairing_allowed(&shared.chrome_control, profile)
    {
        return;
    }
    let name = browser.short_name();
    if decline_removed(shared, id, profile, name) {
        return;
    }
    let mut pending = shared
        .chrome_control
        .owner
        .approvals
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    pending.retain(|_, request| request.opened.elapsed() < LIFE);
    if pending.contains_key(id) || pending.len() >= 128 {
        return;
    }
    shared
        .chrome_control
        .owner
        .approvals
        .forget_declined(profile);
    let generation = shared.chrome_control.browser_generation(browser);
    let weak = Arc::downgrade(shared);
    let request_id = id.to_owned();
    let profile_id = profile.to_owned();
    let profile_generation = shared
        .chrome_control
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .generation(profile);
    let task = tokio::spawn(async move {
        let Some(shared) = weak.upgrade() else { return };
        // `seen`: the folder the dialog read the profile's name from, recorded if the person approves.
        let (request, seen) = dialog(&shared, &profile_id, name).await;
        let confirmation = shared.confirmer.begin(request);
        // Do not keep the core alive just because a dialog is unanswered.
        drop(shared);
        let answer = tokio::time::timeout(LIFE, confirmation).await;
        let Some(shared) = weak.upgrade() else { return };
        let _held = shared.chrome_control.lifecycle.lock().await;
        if shared.chrome_control.browser_generation(browser) != generation
            || shared
                .chrome_control
                .profiles
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .generation(&profile_id)
                != profile_generation
            || !super::profiles::pairing_allowed(&shared.chrome_control, &profile_id)
        {
            return;
        }
        let approved = matches!(answer, Ok(Confirmation::Approved));
        if approved
            && let Err(message) = super::profiles::reapprove(&shared.chrome_control, &profile_id)
        {
            shared.emit(crate::CoreEvent::Alert {
                session_id: None,
                kind: crate::event::AlertKind::BrowserPairingFailed,
                message,
            });
            return;
        }
        let saved = shared
            .chrome_control
            .owner
            .rpc(
                "owner.pairing.approve",
                json!({"requestId":request_id,"approved":approved}),
            )
            .await;
        answered(&shared, profile_id, answer.ok(), saved.as_ref(), name, seen);
    });
    pending.insert(
        id.into(),
        Pending {
            profile: profile.into(),
            opened: Instant::now(),
            task: task.abort_handle(),
        },
    );
}

/// A request from an identity the person removed: the helper hears "Not now" without a dialog, and the card offers
/// Connect (`not_now_card`), which lets it ask again. Returns false for any other identity.
fn decline_removed(shared: &Arc<Shared>, id: &str, profile: &str, name: &'static str) -> bool {
    if !shared.chrome_control.owner.approvals.quiet(profile) {
        return false;
    }
    let weak = Arc::downgrade(shared);
    let (request_id, profile_id) = (id.to_owned(), profile.to_owned());
    tokio::spawn(async move {
        let Some(shared) = weak.upgrade() else { return };
        let saved = shared
            .chrome_control
            .owner
            .rpc(
                "owner.pairing.approve",
                json!({"requestId":request_id,"approved":false}),
            )
            .await;
        answered(
            &shared,
            profile_id,
            Some(Confirmation::Declined),
            saved.as_ref(),
            name,
            None,
        );
    });
    true
}

/// The helper's reply to the person's answer: an approval saved pairs the identity, a Chrome/Edge "Not now" it
/// recorded waits for the person's Connect (`ask_again`), an approval it could not save is reported. `found` is the
/// folder the dialog proved for the identity (`profile_presence::Asking`).
fn answered(
    shared: &Arc<Shared>,
    profile_id: String,
    answer: Option<Confirmation>,
    saved: Option<&Value>,
    name: &str,
    found: Option<Found>,
) {
    let approved = matches!(answer, Some(Confirmation::Approved));
    if approved && saved.is_some_and(|result| result["approved"] == true) {
        approved_pairing(shared, &profile_id, found);
    } else if approved {
        shared.emit(crate::CoreEvent::Alert {
            session_id: None,
            kind: crate::event::AlertKind::BrowserPairingFailed,
            message: format!("{name} could not connect. Restart Vibe Work and try again."),
        });
    } else if matches!(answer, Some(Confirmation::Declined))
        && matches!(
            Browser::from_profile(&profile_id),
            Browser::Chrome | Browser::Edge
        )
        && saved.is_some()
    {
        shared
            .chrome_control
            .owner
            .approvals
            .declined()
            .insert(profile_id);
        super::owner::publish(shared);
    }
}

/// The native pairing dialog for one asking identity, and the folder its name was read from (`dialog_name`).
async fn dialog(
    shared: &Arc<Shared>,
    profile: &str,
    browser: &str,
) -> (ConfirmationRequest, Option<Found>) {
    let (asking, found) = dialog_name(shared, profile, browser).await;
    let request = ConfirmationRequest {
        requester: Requester::Core,
        method: BROWSER_PAIRING_DIALOG_METHOD.into(),
        params: json!({}),
        summary: format!(
            "Connect {asking} to Vibe Work?\n\n{browser} needs your approval. Choose Connect to allow this connection."
        ),
        always: None,
    };
    (request, found)
}

/// "Chrome profile “Work”" when the asking profile's name can be read from the browser's files in time, else
/// "Chrome": with several profiles the person needs to know which one asks. Also returns the folder that read proved.
async fn dialog_name(
    shared: &Arc<Shared>,
    profile: &str,
    browser: &str,
) -> (String, Option<Found>) {
    let (shared, id) = (shared.clone(), profile.to_owned());
    let read = tokio::task::spawn_blocking(move || {
        super::profile_presence::asking(&shared.chrome_control, &id)
    });
    match tokio::time::timeout(NAME_WAIT, read).await {
        Ok(Ok(asking)) => {
            let text = match asking.name {
                Some(profile) => format!("{browser} profile \u{201c}{profile}\u{201d}"),
                None => browser.to_owned(),
            };
            (text, asking.found)
        }
        _ => (browser.to_owned(), None),
    }
}

/// The helper saved the approved pin: the inventory records the pairing with the folder the dialog proved
/// (`profiles::paired`) and Settings refreshes.
fn approved_pairing(shared: &Shared, profile_id: &str, found: Option<Found>) {
    super::profiles::paired(&shared.chrome_control, profile_id, found);
    if Browser::from_profile(profile_id) == Browser::Safari {
        shared
            .chrome_control
            .owner
            .safari_approved
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    super::owner::publish(shared);
}

/// What the person sees after Connect asked again: the question comes with the extension's next reconnect.
pub(super) const ASKED_AGAIN: &str = "Vibe Work asks again when {browser} reconnects, within half a minute. To be asked at once, click Check connection in the Vibe Work extension.";

/// The person's Connect for a browser with a "Not now" identity: the helper forgets that answer
/// (`owner.profileUnpair` of an identity that holds no pin), so the extension's next reconnect asks once more.
/// Connect also ends the quiet of this browser's removed identities. Returns the hint for the person, or `None` when
/// no such identity waits.
pub(super) async fn ask_again(
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
        .filter(|profile| unpinned(chrome, profile))
        .collect();
    if waiting.is_empty() {
        return None;
    }
    for profile in waiting {
        let forgotten = chrome
            .owner
            .rpc("owner.profileUnpair", json!({"profileId":profile}))
            .await;
        if forgotten.is_some_and(|result| result["unpaired"] == true) {
            chrome.owner.approvals.forget_declined(&profile);
        }
    }
    super::owner::publish(shared);
    Some(browser.words(ASKED_AGAIN))
}

/// Only an identity Vibe Work does not hold as paired (never paired, or removed) is asked again this way: forgetting
/// the answer of a paired identity's key change would also end its current pairing.
fn unpinned(chrome: &super::ChromeControl, profile: &str) -> bool {
    super::profiles::pairing_allowed(chrome, profile)
        && chrome
            .profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .records
            .get(&super::profiles::canonical(profile))
            .is_none_or(|record| record.removed)
}

/// The browser card while an identity of this browser waits after "Not now": its Connect asks again.
pub(super) fn not_now_card(
    chrome: &super::ChromeControl,
    view: &mut super::AddonView,
    browser: Browser,
) {
    use super::AddonState as S;
    let waiting: Vec<String> = chrome
        .owner
        .approvals
        .declined_for(browser)
        .into_iter()
        .filter(|profile| unpinned(chrome, profile))
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
    let waits = if removed {
        browser.words(&format!("The user removed {a} {{browser}} profile in Vibe Work while its extension is still connected in {{browser}}. Ask the user to click Connect {{browser}} in Settings > Browser to pair it again."))
    } else {
        browser.words("A {browser} profile asked to connect to Vibe Work and the user chose Not now. Ask the user to click Connect {browser} in Settings > Browser to be asked again.")
    };
    if view.state == super::AddonState::Connected {
        // Another profile of this browser stays connected: the agent keeps that fact and also learns of the
        // waiting profile. The card's label stays Connected; its step line below tells the person.
        view.instruction = format!("{} {waits}", view.instruction);
    } else {
        view.label = browser.words("Waiting for your approval");
        view.instruction = waits;
    }
    view.action = Some(super::addon::AddonAction::Connect);
    view.step = Some(if removed {
        browser.words(&format!(
            "You removed {a} {{browser}} profile that is still connected in {{browser}}. Click Connect {{browser}} to pair it again."
        ))
    } else {
        browser.words(
            "A {browser} profile asked to connect and you chose Not now. Click Connect {browser} to be asked again.",
        )
    });
}

/// The identities of this browser that Vibe Work knows (saved or connected), for a whole-browser Remove.
pub(super) fn removed_browser(chrome: &super::ChromeControl, browser: Browser) {
    let mut known: Vec<String> = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
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
