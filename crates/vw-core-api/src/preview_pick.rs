//! "Point Vibe at this" for a preview element: the same pick script the page viewer uses (vw-pages
//! `pick`), run natively in the session's own preview (engine), polled until the user selects or clicks.
//!
//! Depends on: vw-preview (`PreviewService::evaluate`), vw-pages (pick scripts and parsing), sha2-free naming via
//! vw-pages' project key. Called by: `previews` (`pick`, `cancelPick` requests).
//! Invariants:
//! - What comes back is untrusted page data (length-capped by `vw_pages::parse_pick`); the UI turns it into a
//!   context chip, never a command.
//! - A pick waits at most [`PICK_LIMIT`] and is cancelled (overlay removed) when it ends without an answer.
//! - The accessor name is derived from the session id (a collision guard, not a secret), so `cancelPick` needs no
//!   stored state.

use std::time::Duration;

use vw_pages::PickAnswer;
use vw_preview::PreviewService;

use crate::error::{CoreError, ErrorCode};

/// Longest wait for the user's click.
const PICK_LIMIT: Duration = Duration::from_secs(60);
/// How often the page is asked.
const PICK_POLL: Duration = Duration::from_millis(200);

fn accessor(session_id: &str) -> String {
    format!(
        "__vw_pick_{}",
        &vw_pages::project_key(std::path::Path::new(&format!("preview-pick\0{session_id}")))[..16]
    )
}

/// Runs a pick in `session_id`'s preview.
pub(crate) async fn pick(
    service: &PreviewService,
    session_id: &str,
) -> Result<vw_pages::Picked, CoreError> {
    let name = accessor(session_id);
    let err = |e: vw_preview::PreviewError| crate::previews::core_error(&e);
    let first = service
        .evaluate(session_id, vw_pages::start_script(&name))
        .await
        .map_err(err)?;
    let mut answer = vw_pages::parse_pick(&first);
    let deadline = tokio::time::Instant::now() + PICK_LIMIT;
    while answer == PickAnswer::Pending && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(PICK_POLL).await;
        let polled = service
            .evaluate(session_id, vw_pages::poll_script(&name))
            .await
            .map_err(err)?;
        answer = vw_pages::parse_pick(&polled);
    }
    if let PickAnswer::Picked(picked) = answer {
        return Ok(picked);
    }
    cancel(service, session_id).await;
    Err(CoreError::new(
        ErrorCode::InvalidParams,
        "nothing was picked (Escape, the page changed, or no click within a minute)",
    ))
}

/// Cancels a pick in `session_id`'s preview (removes the overlay); errors are ignored.
pub(crate) async fn cancel(service: &PreviewService, session_id: &str) {
    let _ = service
        .evaluate(session_id, vw_pages::cancel_script(&accessor(session_id)))
        .await;
}
