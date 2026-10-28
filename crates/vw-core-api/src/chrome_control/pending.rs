//! Chrome cards hold their hook until answered; dropping the caller closes the card.
//! Uses the shared ask registry and existing Chrome publisher without retaining retry receipts.
use super::{ChromeAskView, check};
use crate::core::Shared;
use std::sync::Arc;
use vw_ask::Outcome;

pub(super) async fn answer(
    shared: &Arc<Shared>,
    session: &str,
    create: impl FnOnce() -> (crate::ask::service::Handle, ChromeAskView),
) -> Result<Outcome, String> {
    let stop = crate::stops::generation(shared, session);
    let (handle, card) = create();
    check::publish(shared, card.clone());
    let closing = check::Closing {
        shared: shared.clone(),
        card,
    };
    let mut handle = handle;
    handle.cancel_on_drop = true;
    crate::stops::opened_after_stop(shared, session, stop, &handle.ask.id);
    let outcome = handle.closed().await;
    drop(closing);
    if matches!(outcome, Outcome::Allowed { .. })
        && crate::stops::generation(shared, session) != stop
    {
        return Err(crate::stops::APPROVAL_ENDED.into());
    }
    Ok(outcome)
}
