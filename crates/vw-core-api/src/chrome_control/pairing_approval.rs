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

/// The person-only native confirmation shared by Chromium browsers. It has no public approval command.
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
    let Some(browser) = super::enroll_browser::Browser::from_report(Some(profile)) else {
        return;
    };
    if !enroll::uuid_shape(id)
        || browser.raw_profile(profile).is_none()
        || params.as_object().is_none_or(|p| p.len() != 2)
    {
        return;
    }
    let (browser, name, enabled) = match browser {
        super::enroll_browser::Browser::Chrome => {
            (Browser::Chrome, "Chrome", shared.chrome_control.enabled())
        }
        super::enroll_browser::Browser::Edge => {
            (Browser::Edge, "Edge", shared.chrome_control.edge_enabled())
        }
    };
    if !enabled {
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
    let generation = shared.chrome_control.browser_generation(browser);
    let weak = Arc::downgrade(shared);
    let request_id = id.to_owned();
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
        if shared.chrome_control.browser_generation(browser) != generation {
            return;
        }
        let approved = matches!(answer, Ok(Confirmation::Approved));
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
