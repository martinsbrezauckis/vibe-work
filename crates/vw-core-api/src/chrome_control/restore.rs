//! Re-confirms an untrusted saved Chrome level above Browse at core start. Depends on the ordinary native
//! confirmer, the Chrome level state and Settings view. Called once by `Core::start`; Browse remains
//! effective until Keep. A Settings change or Remove invalidates the pending restore, so a late Keep cannot raise it.
use std::sync::Arc;

use serde_json::json;
use vw_ask::ChromeSettings;
use vw_gate::{Confirmation, ConfirmationRequest, Requester};

use crate::core::{Core, Shared};

/// Native confirmation for a Chrome Control level saved above Browse; an owner Debug API may answer it.
pub const CHROME_RESTORE_DIALOG_METHOD: &str = vw_ask::chrome::RESTORE_KIND;

/// Registers the native ask before `Core::start` returns, then lets the app finish starting while the person decides.
pub(crate) fn start(shared: &Arc<Shared>) {
    let Some(saved) = shared.chrome_control.current().restore else {
        return;
    };
    let asking = shared.confirmer.begin(ConfirmationRequest {
        method: CHROME_RESTORE_DIALOG_METHOD.to_owned(),
        summary: format!(
            "Chrome Control is set to {}. Keep it?",
            super::text::name(saved.level.id())
        ),
        params: json!({"level": saved.level}),
        // Saved disk state is not consent: startup must wait for a person or an authenticated owner answer.
        requester: Requester::Window,
        always: None,
    });
    let shared = shared.clone();
    tokio::spawn(async move {
        let keep = asking.await == Confirmation::Approved;
        let chrome = &shared.chrome_control;
        {
            let mut state = chrome.current();
            let Some(saved) = state.restore.take() else {
                return;
            };
            let next = if keep {
                saved
            } else {
                ChromeSettings::default()
            };
            // The write and memory update share the Settings lock. Nothing can interleave a new choice here.
            if let Some(dir) = chrome.dir().filter(|_| !keep)
                && let Err(error) = super::store::save(dir, next)
            {
                tracing::warn!(%error, "Chrome Control's re-confirmed level could not be saved; keeping Browse");
                return;
            }
            state.effective = next;
        }
        let core = Core {
            shared: shared.clone(),
        };
        shared.emit(crate::CoreEvent::ChromeControl {
            view: core.chrome_view(),
        });
    });
}
