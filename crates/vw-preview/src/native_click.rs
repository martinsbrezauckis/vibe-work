//! Host-authorized native pointer input on a mounted preview. Depends on the engine and mount admission;
//! browser actions call this only after their live permission/confirmation checks. Never remounts or shows a view.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, atomic::AtomicBool};

use crate::{EngineError, FrameAnswer, MountId, PersonInterval, PreviewError, PreviewService};

/// A point in the top document's CSS viewport, after all ancestor-frame offsets.
/// The native engine maps viewport fractions to its current widget pixels/points, including zoom and DPI.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NativeClickPoint {
    /// CSS x from the viewport's left edge.
    pub x: f64,
    /// CSS y from the viewport's top edge.
    pub y: f64,
    /// CSS viewport width used by the live page measurement.
    pub viewport_width: f64,
    /// CSS viewport height used by the live page measurement.
    pub viewport_height: f64,
}

/// Host-only validation and dispatch evidence; browser tool arguments cannot supply these fields.
#[derive(Clone, Default)]
pub struct NativeClickGuard {
    /// Host-built frame validation request checked in an isolated world immediately before dispatch.
    pub validation: Option<String>,
    /// Set when a native press is sent; survives cancellation of the awaiting action.
    pub sent: Arc<AtomicBool>,
    /// Host permission recheck on the native UI thread immediately before input; never supplied by the page.
    pub allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// The engine arms this on the view's native thread before the receipt and admits input against it; the
    /// person's own presses and releases in the view disturb it until the receipt is collected.
    pub person: Arc<PersonInterval>,
}

/// Keeps checked input and its event receipt on the same mount, excluding another native action.
/// Dropping the lease releases the session after confirmation or cancellation.
#[must_use]
pub struct NativeClickLease<'a> {
    service: &'a PreviewService,
    mount: MountId,
    frames: BTreeMap<String, u64>,
    person: Arc<PersonInterval>,
    _activity: tokio::sync::OwnedMutexGuard<()>,
}

impl NativeClickLease<'_> {
    /// Reads or clears event evidence without releasing the input session lock.
    ///
    /// # Errors
    /// A replaced mount or unavailable/failed isolated frame evaluator.
    pub async fn evaluate_frames(&self, script: String) -> Result<Vec<FrameAnswer>, PreviewError> {
        if self.service.page_mount(&self.mount.session_id).as_ref() != Some(&self.mount) {
            return Err(EngineError::NotMounted.into());
        }
        let mut answers = self
            .service
            .bounded(
                "native click receipt",
                self.service.engine.evaluate_frames(
                    &self.mount,
                    None,
                    crate::detached_aware(&script),
                ),
            )
            .await?
            .ok_or_else(|| {
                EngineError::Unsupported("native click target confirmation is unavailable".into())
            })?;
        // Read after the answer: a press of the person's that reached these documents passed the host's view
        // first, so it is visible here. The person's click is never taken for the agent's.
        self.person
            .undisturbed()
            .map_err(|text| EngineError::Platform(text.into()))?;
        // Compare native metadata before applying read policy: even an unreadable new frame invalidates input.
        if frame_registry(&answers).ok().as_ref() != Some(&self.frames) {
            return Err(EngineError::Platform(
                "native input was sent; the page changed while clicking, so earlier refs no longer match; read the page again"
                    .into(),
            )
            .into());
        }
        let page = self
            .service
            .page_accessor(&self.mount.session_id)
            .ok_or(EngineError::NotMounted)?;
        // An unobserved frame can move under the point. Its label is still subject to read policy.
        for answer in &mut answers {
            if self
                .service
                .reading_allowed(&self.mount.session_id, &page, &answer.url)
                .is_err()
            {
                answer.value = "null".into();
            }
        }
        Ok(answers)
    }
}

/// How long a checked click waits for a removed frame's lingering handle to leave the registry before it refuses.
const DETACHED_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
/// Pause between two registry snapshots while such a handle is still there.
const DETACHED_RETRY: std::time::Duration = std::time::Duration::from_millis(200);
/// The refusal while a removed frame's handle stays in the registry: no input was sent.
const STILL_CHANGING: &str = "Not run: the page is still changing; read the page again";

fn frame_registry(answers: &[FrameAnswer]) -> Result<BTreeMap<String, u64>, PreviewError> {
    let mut frames = BTreeMap::new();
    for answer in answers {
        // A removed frame's handle is no frame of the page: it neither arms nor changes the registry.
        if answer.value == crate::DETACHED_FRAME {
            continue;
        }
        // A frame whose script cannot run cannot report where the click's events went: no checked click then.
        if answer.value == crate::UNREADABLE_FRAME {
            return Err(EngineError::Platform(crate::UNREPORTING_FRAME_REFUSAL.into()).into());
        }
        let generation = answer.generation.ok_or_else(|| {
            EngineError::Unsupported("native frame registry generation is unavailable".into())
        })?;
        if frames.insert(answer.id.clone(), generation).is_some() {
            return Err(EngineError::Platform("frame identity is ambiguous".into()).into());
        }
    }
    if frames.is_empty() {
        return Err(EngineError::Platform("native frame registry is unavailable".into()).into());
    }
    Ok(frames)
}

impl PreviewService {
    /// Injects into exactly the admitted mount; the engine must refuse a hidden view or a foreign target window.
    /// The caller supplies host authorization, never an agent-supplied coordinate, and holds the returned
    /// lease until the isolated event receipt is confirmed.
    ///
    /// # Errors
    /// Closed/replaced mount, unsupported engine, hidden/foreign window, or native dispatch failure.
    pub async fn native_click_on_mount(
        &self,
        id: &MountId,
        point: NativeClickPoint,
        validation: String,
        sent: Arc<AtomicBool>,
        allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<NativeClickLease<'_>, PreviewError> {
        let activity = self.session_lock(&id.session_id).await;
        if self.page_mount(&id.session_id).as_ref() != Some(id) {
            return Err(PreviewError::Engine(EngineError::NotMounted));
        }
        crate::checked_frame_request(&validation, self.engine.isolated_frame_input())
            .map_err(EngineError::Platform)?;
        // Snapshot every native frame before any receipt is armed; a generation also catches transient frames
        // and navigation that leaves the same routing handle or URL. Hold it through final collection. A removed
        // frame whose handle still answers from its detached document (`WebKit` on macOS) leaves the registry later
        // and changes its generation, which would void the click after input: wait for it to go, else refuse first.
        let started = std::time::Instant::now();
        let answers = loop {
            let answers = self
                .bounded(
                    "native click frame registry",
                    self.engine
                        .evaluate_frames(id, None, crate::detached_aware("null")),
                )
                .await?
                .ok_or_else(|| {
                    EngineError::Unsupported("native frame registry is unavailable".into())
                })?;
            if !answers.iter().any(|a| a.value == crate::DETACHED_FRAME) {
                break answers;
            }
            if started.elapsed() >= DETACHED_WAIT {
                return Err(EngineError::Platform(STILL_CHANGING.into()).into());
            }
            tokio::time::sleep(DETACHED_RETRY).await;
        };
        let frames = frame_registry(&answers)?;
        let person = Arc::new(PersonInterval::default());
        self.touch_pool(&id.session_id);
        self.bounded(
            "native click",
            self.engine.native_click(
                id,
                point,
                NativeClickGuard {
                    validation: Some(validation),
                    sent,
                    allowed,
                    person: person.clone(),
                },
            ),
        )
        .await?;
        Ok(NativeClickLease {
            service: self,
            mount: id.clone(),
            frames,
            person,
            _activity: activity,
        })
    }
}
