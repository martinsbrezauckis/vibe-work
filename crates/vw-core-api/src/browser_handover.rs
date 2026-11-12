//! Hands a chat's browsers back while its question waits for a person.
//! Uses the ask registry and existing cancellation/release paths; no extension messages or page heuristics.
use crate::core::Shared;

pub(crate) fn opened(shared: &Shared, session: &str) {
    // Called under the ask registry lock before the question is published: only release state here.
    shared.chrome_control.handover(session);
    if let Some(hub) = &shared.preview {
        for page in std::iter::once(session.to_owned()).chain(hub.service.web_tabs(session)) {
            hub.browser.sessions().cancel_actions(&page);
            hub.browser.sessions().cancel_confirmations(&page);
        }
    }
}

pub(crate) fn check(shared: &Shared, session: &str) -> Result<(), String> {
    match shared.asks.question_hold(session) {
        Some(reason) => Err(reason.to_owned()),
        None => Ok(()),
    }
}
