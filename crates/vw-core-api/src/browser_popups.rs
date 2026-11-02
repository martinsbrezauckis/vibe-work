//! Routes page pop-ups into their chat's agent tabs. Depends on current tab ownership, Stop and `CoreNav` policy.
//! Installed by preview startup; the native engine still refuses unmanaged windows and opener access.
use crate::core::Shared;
use std::sync::Arc;
use vw_browser::{Actor, Navigate as _, Outcome};
use vw_preview::MountId;

fn authorized(shared: &Shared, owner: &str, source: &MountId) -> bool {
    shared.store.view(owner).is_some()
        && shared.preview.as_ref().is_some_and(|hub| {
            crate::browser_tabs::agent_page_allowed(&hub.service, owner, &source.session_id)
                && !hub.browser.sessions().stopped(owner)
                && !hub.browser.sessions().stopped(&source.session_id)
        })
}

fn allowed(shared: &Shared, owner: &str, source: &MountId) -> bool {
    authorized(shared, owner, source)
        && shared
            .preview
            .as_ref()
            .is_some_and(|hub| hub.service.page_mount(&source.session_id).as_ref() == Some(source))
}

pub(crate) fn install(shared: &Arc<Shared>) {
    let Some(hub) = &shared.preview else {
        return;
    };
    let runtime = tokio::runtime::Handle::current();
    let weak = Arc::downgrade(shared);
    let pending = Arc::new(tokio::sync::Semaphore::new(8));
    hub.service.on_new_window(move |source, url| {
        let Some(shared) = weak.upgrade() else {
            return false;
        };
        let Some(hub) = &shared.preview else {
            return false;
        };
        let Some(owner) = hub.service.owner_session_id(&source.session_id) else {
            return false;
        };
        if !allowed(&shared, &owner, &source) {
            return false;
        }
        let Ok(slot) = pending.clone().try_acquire_owned() else {
            return false;
        };
        let mut owner_cancel = hub.browser.sessions().cancel_signal(&owner);
        let mut page_cancel = hub.browser.sessions().cancel_signal(&source.session_id);
        owner_cancel.borrow_and_update();
        page_cancel.borrow_and_update();
        runtime.spawn(async move {
            let _slot = slot;
            if !allowed(&shared, &owner, &source) {
                return;
            }
            let Some(hub) = &shared.preview else {
                return;
            };
            let Ok(key) = hub.service.create_web_tab(&owner) else {
                return;
            };
            let nav = crate::browser::CoreNav {
                shared: shared.clone(),
                strict: true,
            };
            let result = tokio::select! {
                result = nav.open(&key, url.as_str()) => result,
                _ = owner_cancel.changed() => Err("Browser action canceled".into()),
                _ = page_cancel.changed() => Err("Browser action canceled".into()),
            };
            let result = result.and_then(|()| {
                // Admission can suspend the opener to free an engine. Its chat authority must still hold.
                if authorized(&shared, &owner, &source) {
                    Ok(())
                } else {
                    Err("Browser opener changed".into())
                }
            });
            let (outcome, detail) = match result {
                Ok(()) => {
                    let _ = hub.service.set_agent_page(&owner, &key);
                    (Outcome::Done, "Opened an agent tab".to_owned())
                }
                Err(error) => {
                    let _ = hub.service.close_web_tab_and_restore(&owner, &key).await;
                    (Outcome::Refused, error)
                }
            };
            hub.browser.sessions().end(
                &source.session_id,
                Actor::Agent,
                "popup",
                url.as_str(),
                outcome,
                &detail,
            );
        });
        true
    });
}
