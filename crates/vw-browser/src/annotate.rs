//! "Annotate this page": the user points at an element or draws a box in the Browser tab,
//! and the answer (what and where, plus a screenshot crop of that area) becomes context for the composer. Same
//! polling shape as "Point Vibe at this" (`preview_pick.rs`), with a box mode and a crop.
//!
//! Depends on: vw-preview (evaluate, screenshot, image helpers), `scripts`, `model` (`parse_answer`). Called by:
//! vw-core-api (`browser annotate` / `cancelAnnotate`), tests.
//! Invariants: the answer is untrusted page data (capped here again); a pick waits at most [`ANNOTATE_LIMIT`] and
//! the overlay is removed when it ends without an answer; the crop is at most the screenshot's size (<= 1280 px).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use vw_preview::{PixelRect, PreviewService, crop, decode, encode};

use crate::model::{is_ref, parse_answer};
use crate::scripts;

/// Longest wait for the user's click or box.
pub const ANNOTATE_LIMIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(200);

/// A rectangle in the page's viewport (CSS pixels).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ViewRect {
    /// Left.
    #[serde(default)]
    pub x: i32,
    /// Top.
    #[serde(default)]
    pub y: i32,
    /// Width.
    #[serde(default)]
    pub width: i32,
    /// Height.
    #[serde(default)]
    pub height: i32,
}

/// What the user pointed at (untrusted page data, capped).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    /// `element` or `box`.
    pub mode: String,
    /// The element's ref, when it is an interactive element.
    #[serde(default)]
    pub reference: Option<String>,
    /// Role.
    #[serde(default)]
    pub role: String,
    /// Accessible name.
    #[serde(default)]
    pub name: String,
    /// Its text (at most 500 characters).
    #[serde(default)]
    pub text: String,
    /// Its DOM path.
    #[serde(default)]
    pub path: String,
    /// The page URL.
    #[serde(default)]
    pub url: String,
    /// Path, query and fragment of the page (the app's route).
    #[serde(default)]
    pub route: String,
    /// A source-file hint the page's build put on the element (`data-insp-path`, ...), else none.
    #[serde(default)]
    pub source: Option<String>,
    /// Where, in the viewport.
    #[serde(default)]
    pub rect: ViewRect,
    /// A PNG of that area (base64), when the screenshot worked.
    #[serde(default)]
    pub crop_png_base64: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    state: String,
    #[serde(default)]
    mode: String,
    #[serde(default, rename = "ref")]
    reference: Option<String>,
    #[serde(default)]
    role: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    route: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    rect: ViewRect,
    #[serde(default)]
    viewport: ViewRect,
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn accessor(session_id: &str) -> String {
    format!(
        "__vw_annotate_{}",
        &crate::observe::hash(&["annotate", session_id])[..12]
    )
}

fn script(page: &str, session_id: &str, op: &str, mode: &str) -> String {
    scripts::annotate(&json!({"page": page, "name": accessor(session_id), "op": op, "mode": mode}))
}

/// The script for a named target: `target` is `{ref}`, `{selector}`, `{x, y}` or (box mode) `{rect}`.
fn target_script(page: &str, session_id: &str, mode: &str, target: &serde_json::Value) -> String {
    scripts::annotate(&json!({
        "page": page, "name": accessor(session_id), "op": "target", "mode": mode, "target": target,
    }))
}

/// Starts the overlay and polls until a person picks, cancels, or a minute passes.
async fn wait_for_pick(
    service: &PreviewService,
    page: &str,
    session_id: &str,
    mode: &'static str,
) -> Result<Raw, String> {
    let ask = |op: &'static str| async move {
        let raw = service
            .evaluate(session_id, script(page, session_id, op, mode))
            .await
            .map_err(|e| e.to_string())?;
        parse_answer::<Raw>(&raw)
    };
    let mut answer = ask("start").await?;
    let deadline = tokio::time::Instant::now() + ANNOTATE_LIMIT;
    while answer.state == "pending" && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(POLL).await;
        answer = ask("poll").await?;
    }
    if answer.state != "picked" {
        cancel(service, page, session_id).await;
        return Err(
            "nothing was annotated (Escape, the page changed, or nothing within a minute)"
                .to_owned(),
        );
    }
    Ok(answer)
}

/// Runs an annotation in `session_id`'s browser (`box_mode`: draw a box; else click an element). With `target` (a
/// request names the element or area, as the Debug API does, because no person points) it answers at once with what
/// a click or a drawn box would have given, and never shows an overlay or waits.
///
/// # Errors
/// A plain-text reason (nothing open, cancelled, no answer within a minute; for a target: nothing there, or an
/// invalid selector).
pub async fn annotate(
    service: &PreviewService,
    page: &str,
    session_id: &str,
    box_mode: bool,
    target: Option<&serde_json::Value>,
) -> Result<Annotation, String> {
    let mode = if box_mode { "box" } else { "element" };
    let answer = match target {
        Some(target) => {
            let raw = service
                .evaluate(session_id, target_script(page, session_id, mode, target))
                .await
                .map_err(|e| e.to_string())?;
            let answer = parse_answer::<Raw>(&raw)?;
            match answer.state.as_str() {
                "picked" => answer,
                "invalid" => return Err("the selector is not a valid CSS selector".to_owned()),
                _ => return Err("no element matches the target".to_owned()),
            }
        }
        None => wait_for_pick(service, page, session_id, mode).await?,
    };
    let mut annotation = Annotation {
        mode: clip(&answer.mode, 10),
        reference: answer.reference.filter(|r| is_ref(r)),
        role: clip(&answer.role, 30),
        name: clip(&answer.name, 160),
        text: clip(&answer.text, 500),
        path: clip(&answer.path, 512),
        url: clip(&answer.url, 500),
        route: clip(&answer.route, 300),
        source: answer.source.map(|s| clip(&s, 200)),
        rect: answer.rect,
        crop_png_base64: None,
    };
    annotation.crop_png_base64 = crop_of(service, session_id, answer.rect, answer.viewport).await;
    Ok(annotation)
}

/// Cancels a running annotation (removes the overlay); errors are ignored.
pub async fn cancel(service: &PreviewService, page: &str, session_id: &str) {
    let _ = service
        .evaluate(session_id, script(page, session_id, "cancel", "element"))
        .await;
}

/// Space around the annotated area in the crop (CSS pixels), so the agent sees the element in its place.
const MARGIN: i32 = 16;

/// A PNG of `rect` plus [`MARGIN`] (viewport CSS pixels) from a fresh screenshot scaled by its width against the
/// viewport's.
async fn crop_of(
    service: &PreviewService,
    session_id: &str,
    rect: ViewRect,
    viewport: ViewRect,
) -> Option<String> {
    let shot = service.screenshot(session_id).await.ok()?;
    let image = decode(&shot.png).ok()?;
    if viewport.width <= 0 || rect.width <= 0 || rect.height <= 0 {
        return None;
    }
    // Integer scaling: screenshot pixels per viewport CSS pixel (the screenshot is at most 1280 px wide).
    let (num, den) = (i64::from(image.width), i64::from(viewport.width));
    let to_px = |v: i32| i64::from(v).saturating_mul(num) / den;
    let area = PixelRect {
        x: to_px(rect.x - MARGIN),
        y: to_px(rect.y - MARGIN),
        width: u32::try_from(to_px(rect.width + 2 * MARGIN).max(1)).ok()?,
        height: u32::try_from(to_px(rect.height + 2 * MARGIN).max(1)).ok()?,
    };
    let cropped = crop(&image, area)?;
    let png = encode(&cropped).ok()?;
    Some(base64_of(&png))
}

fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The recheck script's answer.
#[derive(Deserialize)]
struct States {
    states: Vec<String>,
}

/// One annotation to check again after the agent's work.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RecheckItem {
    /// The annotated element's ref, when it had one.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub reference: Option<String>,
    /// Its text when annotated.
    #[serde(default)]
    pub text: String,
}

/// After the agent worked on annotations: optionally reloads the page (waits for the load, bounded), then says per
/// item `changed`, `unchanged` or `missing`.
///
/// # Errors
/// A plain-text reason (nothing open, the page did not answer).
pub async fn recheck(
    service: &PreviewService,
    session_id: &str,
    items: &[RecheckItem],
    reload: bool,
) -> Result<Vec<String>, String> {
    if reload {
        let before = service.load_state(session_id).map_or(0, |s| s.loads);
        service
            .history(session_id, vw_preview::HistoryStep::Reload)
            .await
            .map_err(|e| e.to_string())?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while tokio::time::Instant::now() < deadline
            && service
                .load_state(session_id)
                .is_none_or(|s| s.loads <= before || s.loading)
        {
            tokio::time::sleep(POLL).await;
        }
    }
    let page = service
        .page_accessor(session_id)
        .ok_or_else(|| crate::NOTHING_OPEN.to_owned())?;
    let wire: Vec<serde_json::Value> = items
        .iter()
        .take(50)
        .map(|i| json!({"ref": i.reference.as_deref().filter(|r| is_ref(r)), "text": clip(&i.text, 500)}))
        .collect();
    let raw = service
        .evaluate(
            session_id,
            scripts::recheck(&json!({"page": page, "items": wire})),
        )
        .await
        .map_err(|e| e.to_string())?;
    let states: States = parse_answer(&raw)?;
    Ok(states
        .states
        .into_iter()
        .map(|s| match s.as_str() {
            "changed" | "unchanged" | "missing" => s,
            _ => "missing".to_owned(),
        })
        .collect())
}
