//! Native person confirmation for an exact helper pairing request. Depends on the owner pipe and ask service;
//! called only by trusted owner notifications, with cancellation on helper loss or browser disable.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use vw_platform::sync::LockRecover;

use serde_json::{Value, json};
use tokio::task::AbortHandle;
use vw_gate::{Confirmation, ConfirmationRequest, Requester};

use super::{agent_browser::Browser, enroll, profile_presence::Found};
use crate::core::Shared;
#[path = "pairing_approval_again.rs"]
mod again;
pub(super) use again::{ask_again, not_now_card, removed_browser};

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
        self.0.lock_or_recover().clear();
        self.declined().clear();
    }

    fn declined(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.1.lock_or_recover()
    }

    fn removed(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.2.lock_or_recover()
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
            .lock_or_recover()
            .retain(|_, pending| !browser.owns_profile(&pending.profile));
        self.declined()
            .retain(|profile| !browser.owns_profile(profile));
    }

    /// A native pairing approval still waits for this identity (an expired one no longer counts).
    pub(super) fn waiting(&self, profile: &str) -> bool {
        self.0.lock_or_recover().values().any(|pending| {
            pending.opened.elapsed() < LIFE && super::profile_ids::same(&pending.profile, profile)
        })
    }

    pub(super) fn cancel_profile(&self, profile: &str) {
        self.0
            .lock_or_recover()
            .retain(|_, pending| !super::profile_ids::same(&pending.profile, profile));
        self.forget_declined(profile);
    }

    pub(super) fn paired(&self, profile: &str) {
        self.forget_declined(profile);
        self.removal_failed(profile);
        for pending in self.0.lock_or_recover().values() {
            if pending.profile == profile {
                pending.task.abort();
            }
        }
    }

    /// A later authenticated status can resolve a person's earlier Not now without a new pairing event.
    /// An explicitly removed profile stays quiet even if its extension still reports a connection.
    pub(super) fn connected(&self, data: &Value) {
        if let Some(profiles) = data["profiles"].as_array() {
            for row in profiles {
                if row["connected"] == true
                    && let Some(profile) = row["profileId"].as_str()
                    && !self.quiet(profile)
                {
                    self.forget_declined(profile);
                }
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
    let mut pending = shared.chrome_control.owner.approvals.0.lock_or_recover();
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
        .lock_or_recover()
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
                .lock_or_recover()
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
