//! Running a pick on an engine mount (the page viewer, or a session's preview) and turning its answer into a page
//! section: the first reported id that exists in the page FILE (labels from the file). Split out of `service`.
//!
//! Depends on: vw-preview (engine seam), `pick` (scripts, parsing), `config`, `viewer` (pick bookkeeping), `page`.
//! Called by: `service`, vw-core-api (preview picks use `run_pick`'s scripts through the preview service).
//! Invariant: a pick that ends without an answer is cancelled (the overlay goes away); every engine call is bounded.

use std::time::Duration;

use vw_preview::{EngineError, EngineFuture, MountId, PreviewEngine};

use crate::config::{PagesConfig, PagesError, PickedSection};
use crate::page::PageFacts;
use crate::paths::PagePath;
use crate::pick::{PickAnswer, Picked};
use crate::viewer::Shared;

async fn bounded<T>(
    limit: Duration,
    what: &str,
    future: EngineFuture<'_, T>,
) -> Result<T, EngineError> {
    tokio::time::timeout(limit, future)
        .await
        .map_err(|_| EngineError::Timeout(what.to_owned()))?
}

/// The section a pick lands in: the first reported id the page file has (labels from the file).
#[must_use]
pub fn section_of(facts: &PageFacts, page: &PagePath, picked: &Picked) -> Option<PickedSection> {
    let section = picked.ids.iter().find(|id| facts.has_section(id))?;
    Some(PickedSection {
        page: page.to_string(),
        section: section.clone(),
        label: facts.section_label(section).unwrap_or_default().to_owned(),
        quote: picked.quote.clone(),
        text: picked.text.clone(),
    })
}

/// Runs a pick on any engine mount (the page viewer, or a session's preview): start, poll until it answers or the
/// time limit, cancel on the way out.
///
/// # Errors
/// [`PagesError::NothingPicked`], engine failures.
pub(crate) async fn pick_on(
    engine: &dyn PreviewEngine,
    id: &MountId,
    config: &PagesConfig,
    shared: &Shared,
) -> Result<Picked, PagesError> {
    let name = crate::pick::accessor_name();
    shared.lock().picking = Some(name.clone());
    let outcome = run_pick(engine, id, config, &name).await;
    if !matches!(outcome, Ok(PickAnswer::Picked(_))) {
        let _ = bounded(
            config.engine_timeout,
            "cancel pick",
            engine.evaluate(id, crate::pick::cancel_script(&name)),
        )
        .await;
    }
    {
        let mut state = shared.lock();
        if state.picking.as_deref() == Some(name.as_str()) {
            state.picking = None;
        }
    }
    match outcome? {
        PickAnswer::Picked(picked) => Ok(picked),
        _ => Err(PagesError::NothingPicked),
    }
}

/// The pick loop without bookkeeping (also used for previews).
///
/// # Errors
/// Engine failures.
pub async fn run_pick(
    engine: &dyn PreviewEngine,
    id: &MountId,
    config: &PagesConfig,
    name: &str,
) -> Result<PickAnswer, EngineError> {
    let first = bounded(
        config.engine_timeout,
        "pick",
        engine.evaluate(id, crate::pick::start_script(name)),
    )
    .await?;
    let mut answer = crate::pick::parse(&first);
    let deadline = tokio::time::Instant::now() + config.pick_timeout;
    while answer == PickAnswer::Pending && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(config.pick_poll).await;
        let polled = bounded(
            config.engine_timeout,
            "pick",
            engine.evaluate(id, crate::pick::poll_script(name)),
        )
        .await?;
        answer = crate::pick::parse(&polled);
    }
    Ok(match answer {
        PickAnswer::Pending => PickAnswer::Cancelled,
        other => other,
    })
}
