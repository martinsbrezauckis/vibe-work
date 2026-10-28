//! "Point Vibe at this" for a preview element: the same pick script the page viewer uses (vw-pages
//! `pick`), run natively in the session's own preview (engine), polled until the user selects or clicks. Also the
//! rules for a pick that names its target instead (a Debug API request has no pointer).
//!
//! Depends on: vw-preview (`PreviewService::evaluate`), vw-pages (pick scripts and parsing), sha2-free naming via
//! vw-pages' project key. Called by: `previews` (`pick`, `cancelPick` requests), `browser` (annotate targets),
//! `pages` (section names), `dispatch` (the Debug API's up-front target check).
//! Invariants:
//! - What comes back is untrusted page data (length-capped by `vw_pages::parse_pick`); the UI turns it into a
//!   context chip, never a command.
//! - A pick waits at most [`PICK_LIMIT`] and is cancelled (overlay removed) when it ends without an answer. A web
//!   page's pick (`tabId`) reads the selection only: a web page never gets the click overlay.
//! - The accessor name is derived from the session id (a collision guard, not a secret), so `cancelPick` needs no
//!   stored state.
//! - A Debug API caller completes a pick without a person: every pick (this preview pick, the Browser panel's
//!   annotation, the Project tab's section pick) takes its target in the request and answers at once with the shape
//!   a person's pick gives. One without a target is `invalidParams` naming the missing field, never a wait:
//!   [`admit`], run by `Core::execute` before any session lease or native work. A window request without a
//!   target is the person's pick, unchanged.

use std::time::Duration;

use serde_json::{Value, json};
use vw_pages::{PickAnswer, PickAt, TargetMiss};
use vw_preview::PreviewService;

use crate::browser_view::BrowserRequest;
use crate::command::{Command, Origin};
use crate::error::{CoreError, ErrorCode};
use crate::pages_view::PagesRequest;
use crate::preview_view::{PickTarget, PreviewRequest};

/// Longest wait for the user's click.
const PICK_LIMIT: Duration = Duration::from_secs(60);
/// How often the page is asked.
const PICK_POLL: Duration = Duration::from_millis(200);
/// Longest CSS selector accepted.
const SELECTOR_MAX: usize = 1000;

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

/// Picks the element `target` names in `session_id`'s preview, at once: no overlay, no waiting.
pub(crate) async fn pick_at(
    service: &PreviewService,
    session_id: &str,
    target: &PickTarget,
) -> Result<vw_pages::Picked, CoreError> {
    let at = preview_at(Some(target))?;
    let reply = service
        .evaluate(
            session_id,
            vw_pages::target_script_with_quote(&at, target.quote.as_deref()),
        )
        .await
        .map_err(|e| crate::previews::core_error(&e))?;
    vw_pages::parse_target(&reply).map_err(|miss| {
        invalid(match miss {
            TargetMiss::NotFound => "no element matches the target",
            TargetMiss::InvalidSelector => "target.selector is not a valid CSS selector",
            TargetMiss::Unreadable => "the page did not answer the pick",
        })
    })
}

/// A `pick` request: a web page's selection (`tab_id`), the element `target` names, or a person's pick.
pub(crate) async fn run(
    service: &PreviewService,
    session_id: &str,
    target: Option<PickTarget>,
    tab_id: Option<String>,
) -> Result<vw_pages::Picked, CoreError> {
    match (target, tab_id) {
        (target, Some(tab)) => pick_in_tab(service, session_id, &tab, target.as_ref()).await,
        (Some(target), None) => pick_at(service, session_id, &target).await,
        (None, None) => pick(service, session_id).await,
    }
}

/// A web page `tab` of `session_id`'s Browser tab, after its own menu's Point Vibe at this: the page's selection
/// now (never an overlay or a wait), or with `target` the element it names (the Debug API's way).
pub(crate) async fn pick_in_tab(
    service: &PreviewService,
    session_id: &str,
    tab: &str,
    target: Option<&PickTarget>,
) -> Result<vw_pages::Picked, CoreError> {
    if service.owner_session_id(tab).as_deref() != Some(session_id)
        || !service.web_tabs(session_id).iter().any(|key| key == tab)
    {
        return Err(invalid(
            "tabId is not a web page of this session's Browser tab",
        ));
    }
    let id = service
        .page_mount(tab)
        .ok_or_else(|| invalid("the web page is not open"))?;
    let script = match target {
        Some(target) => {
            vw_pages::target_script_with_quote(&preview_at(Some(target))?, target.quote.as_deref())
        }
        None => vw_pages::selection_script(),
    };
    let reply = service
        .evaluate_on_mount(&id, script)
        .await
        .map_err(|e| crate::previews::core_error(&e))?;
    match vw_pages::parse_pick(&reply) {
        PickAnswer::Picked(picked) => Ok(picked),
        _ if target.is_some() => Err(invalid("no element matches the target")),
        _ => Err(invalid("Select text on the page first.")),
    }
}

/// The Copy action's named read, limited to a web tab owned by this session.
pub(crate) async fn copy_text(
    service: &PreviewService,
    session_id: &str,
    tab: &str,
    target: &PickTarget,
) -> Result<String, CoreError> {
    let at = preview_at(Some(target))?;
    if service.owner_session_id(tab).as_deref() != Some(session_id)
        || !service.web_tabs(session_id).iter().any(|key| key == tab)
    {
        return Err(invalid(
            "tabId is not a web page of this session's Browser tab",
        ));
    }
    let id = service
        .page_mount(tab)
        .ok_or_else(|| invalid("the web page is not open"))?;
    let reply = service
        .evaluate_on_mount(
            &id,
            vw_pages::copy_target_script(&at, target.quote.as_deref()),
        )
        .await
        .map_err(|e| crate::previews::core_error(&e))?;
    vw_pages::parse_named_copy(&reply).ok_or_else(|| invalid("no text matches the target"))
}

/// Cancels a pick in `session_id`'s preview (removes the overlay); errors are ignored.
pub(crate) async fn cancel(service: &PreviewService, session_id: &str) {
    let _ = service
        .evaluate(session_id, vw_pages::cancel_script(&accessor(session_id)))
        .await;
}

fn invalid(text: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, text)
}

/// The point of a target, when it names one (both coordinates, finite).
fn point(target: &PickTarget) -> Result<Option<(f64, f64)>, CoreError> {
    match (target.x, target.y) {
        (None, None) => Ok(None),
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok(Some((x, y))),
        _ => Err(invalid(
            "target.x and target.y name a point: give both, as numbers",
        )),
    }
}

fn selector(target: &PickTarget) -> Result<Option<&str>, CoreError> {
    if target
        .quote
        .as_ref()
        .is_some_and(|q| q.is_empty() || q.chars().count() > 100_000)
    {
        return Err(invalid("target.quote must contain 1 to 100000 characters"));
    }
    match target.selector.as_deref() {
        None => Ok(None),
        Some(s) if !s.trim().is_empty() && s.len() <= SELECTOR_MAX => Ok(Some(s)),
        Some(_) => Err(invalid(
            "target.selector must be a CSS selector of at most 1000 characters",
        )),
    }
}

/// What a preview pick takes: `selector`, or `x` and `y`; with none, a Debug API request is refused by name.
pub(crate) fn preview_at(target: Option<&PickTarget>) -> Result<PickAt, CoreError> {
    const NAMES: &str =
        "a pick from the Debug API names its target: target.selector, or target.x and target.y";
    let target = target.ok_or_else(|| invalid(NAMES))?;
    if target.reference.is_some() || target.rect.is_some() {
        return Err(invalid(
            "target.ref and target.rect are for browser annotate; a preview pick takes target.selector, or target.x and target.y",
        ));
    }
    match (selector(target)?, point(target)?) {
        (Some(s), None) => Ok(PickAt::Selector(s.to_owned())),
        (None, Some((x, y))) => Ok(PickAt::Point { x, y }),
        (None, None) => Err(invalid(NAMES)),
        (Some(_), Some(_)) => Err(invalid(
            "name one target: target.selector, or target.x and target.y",
        )),
    }
}

/// What an annotation takes, as the page script's `target` object: for an element `ref`, `selector`, or `x` and
/// `y`; in box mode `rect`. With none, a Debug API request is refused by name.
pub(crate) fn annotate_target(
    mode: Option<&str>,
    target: Option<&PickTarget>,
) -> Result<Value, CoreError> {
    let boxed = mode == Some("box");
    let names = if boxed {
        "a box annotation from the Debug API names its area: target.rect (x, y, width, height)"
    } else {
        "an annotation from the Debug API names its target: target.ref, target.selector, or target.x and target.y"
    };
    let target = target.ok_or_else(|| invalid(names))?;
    if target.quote.is_some() {
        return Err(invalid("target.quote is for preview picks and Copy"));
    }
    if boxed {
        let rect = target.rect.ok_or_else(|| invalid(names))?;
        let sane = [rect.x, rect.y, rect.width, rect.height]
            .iter()
            .all(|v| v.is_finite())
            && rect.width > 0.0
            && rect.height > 0.0;
        if !sane
            || target.selector.is_some()
            || target.reference.is_some()
            || point(target)?.is_some()
        {
            return Err(invalid(
                "in box mode name only target.rect, with a positive width and height",
            ));
        }
        return Ok(
            json!({"rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}}),
        );
    }
    if target.rect.is_some() {
        return Err(invalid(
            "target.rect is for mode box; an element takes target.ref, target.selector, or target.x and target.y",
        ));
    }
    let chosen = [
        target.reference.is_some(),
        target.selector.is_some(),
        target.x.is_some() || target.y.is_some(),
    ]
    .iter()
    .filter(|named| **named)
    .count();
    match chosen {
        0 => return Err(invalid(names)),
        1 => {}
        _ => {
            return Err(invalid(
                "name one target: target.ref, target.selector, or target.x and target.y",
            ));
        }
    }
    if let Some(reference) = &target.reference {
        if !vw_browser::model::is_ref(reference) {
            return Err(invalid(
                "target.ref is not an element ref (e and eight hex digits, from a read of the page)",
            ));
        }
        return Ok(json!({"ref": reference}));
    }
    if let Some(s) = selector(target)? {
        return Ok(json!({"selector": s}));
    }
    let (x, y) = point(target)?.ok_or_else(|| invalid(names))?;
    Ok(json!({"x": x, "y": y}))
}

/// The section a Debug API section pick names; refused by name when absent.
pub(crate) fn section_name(section: Option<&str>) -> Result<&str, CoreError> {
    match section.map(str::trim) {
        Some(name) if !name.is_empty() => Ok(name),
        _ => Err(invalid(
            "a section pick from the Debug API names the section: section (a section or step id of the shown page)",
        )),
    }
}

/// For the Debug API: refuses a pick request that names no valid target, before the session is read or anything
/// native starts. Other requests, other origins and the window's own picks (which have a person) pass.
pub(crate) fn admit(origin: Origin, command: &Command) -> Result<(), CoreError> {
    if origin == Origin::DebugApi {
        check_named(command)
    } else {
        Ok(())
    }
}

fn check_named(command: &Command) -> Result<(), CoreError> {
    match command {
        Command::Preview {
            request: PreviewRequest::Pick { target, .. },
        } => preview_at(target.as_ref()).map(drop),
        Command::Preview {
            request: PreviewRequest::CopyText { target, .. },
        } => preview_at(Some(target)).map(drop),
        Command::Browser {
            request: BrowserRequest::Annotate { mode, target, .. },
        } => annotate_target(mode.as_deref(), target.as_ref()).map(drop),
        Command::Pages {
            request: PagesRequest::PickSection { section, .. },
        } => section_name(section.as_deref()).map(drop),
        _ => Ok(()),
    }
}
