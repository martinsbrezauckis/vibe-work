//! Native person confirmation for an exact helper pairing request. Depends on the owner pipe and ask service;
//! called only by trusted owner notifications, with cancellation on helper loss or browser disable.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tokio::task::AbortHandle;
use vw_gate::{Confirmation, ConfirmationRequest, Requester};

use super::{agent_browser::Browser, enroll};
use crate::core::Shared;

/// The person-only native confirmation shared by all connected browsers. It has no public approval command.
pub const BROWSER_PAIRING_DIALOG_METHOD: &str = "chrome/control/pair";
const LIFE: Duration = Duration::from_secs(10 * 60);

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

#[derive(Default)]
pub(super) struct Approvals(Mutex<BTreeMap<String, Pending>>);

impl Approvals {
    pub(super) fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    pub(super) fn cancel_browser(&self, browser: Browser) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, pending| !browser.owns_profile(&pending.profile));
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
    }

    pub(super) fn paired(&self, profile: &str) {
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
        let confirmation = shared.confirmer.begin(ConfirmationRequest {
            requester: Requester::Core,
            method: BROWSER_PAIRING_DIALOG_METHOD.into(),
            params: json!({}),
            summary: format!("Connect {name} to Vibe Work?\n\n{name} needs your approval. Choose Connect to allow this connection."),
            always: None,
        });
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
        if approved && !saved.is_some_and(|result| result["approved"] == true) {
            shared.emit(crate::CoreEvent::Alert {
                session_id: None,
                kind: crate::event::AlertKind::BrowserPairingFailed,
                message: format!("{name} could not connect. Restart Vibe Work and try again."),
            });
        }
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
