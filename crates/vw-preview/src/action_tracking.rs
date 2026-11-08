//! Host-only action records shared by built-in input and external-browser receipts. No page text or typed values.
//! Producers supply source identity and CSS geometry; the host overlay consumes normalized points, and optional
//! subscribers can correlate actions on one monotonic clock. No history, page injection or global input hooks.

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// The maximum lifetime of an action marker, measured from host receipt time.
pub const MARKER_LIFETIME: Duration = Duration::from_millis(700);

/// Input kind, without the input's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind {
    /// A pointer press and release.
    Click,
    /// Text entered into a control; text is never retained here.
    Type,
    /// A checked state changed.
    Check,
    /// An option selected.
    Select,
    /// A key sent; key content is never retained here.
    Key,
    /// A viewport scrolled.
    Scroll,
}

/// Browser identity comes from the host's binding, never a page field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceBrowser {
    /// The app's native browser surface.
    BuiltIn,
    /// A bound Chrome profile.
    Chrome,
    /// A bound Edge profile.
    Edge,
    /// A bound Safari profile.
    Safari,
    /// A bound Firefox profile.
    Firefox,
}

/// Exact surface identity. Missing external tab/document identity never means the selected app tab.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionSource {
    /// Browser owning the action.
    pub browser: SourceBrowser,
    /// Native mount label, or external tab id. No URL-based matching.
    pub view_id: Option<String>,
    /// Document identity when supplied by the input receipt.
    pub document_id: Option<String>,
    /// External browser profile; absent for the built-in browser.
    pub profile_id: Option<String>,
    /// External browser lease; absent for the built-in browser.
    pub browser_session_id: Option<String>,
}

/// A point in the source's top-level CSS viewport.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionPoint {
    /// Horizontal CSS position.
    pub x: f64,
    /// Vertical CSS position.
    pub y: f64,
}

/// A CSS rectangle in that same viewport; never screen or child-frame coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionBox {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// Geometry may be absent in older external receipts. Absence is retained, never guessed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionGeometry {
    /// Actual dispatched point when known.
    pub point: Option<ActionPoint>,
    /// Observed target rectangle when known.
    pub target_box: Option<ActionBox>,
    /// Top-level CSS viewport width and height from the same observation.
    pub viewport: Option<(f64, f64)>,
}

impl ActionGeometry {
    /// Validated viewport fraction for the marker. A target-only action uses its visible intersection's centre.
    #[must_use]
    pub fn normalized_point(self) -> Option<(f64, f64)> {
        let (width, height) = self.viewport?;
        if ![width, height]
            .into_iter()
            .all(|v| v.is_finite() && v > 0.0)
        {
            return None;
        }
        let point = if let Some(point) = self.point {
            point
        } else {
            let rect = self.target_box?;
            if ![rect.x, rect.y, rect.width, rect.height]
                .into_iter()
                .all(f64::is_finite)
                || rect.width <= 0.0
                || rect.height <= 0.0
            {
                return None;
            }
            let (left, top) = (rect.x.max(0.0), rect.y.max(0.0));
            let (right, bottom) = (
                (rect.x + rect.width).min(width),
                (rect.y + rect.height).min(height),
            );
            if right <= left || bottom <= top {
                return None;
            }
            ActionPoint {
                x: left.midpoint(right),
                y: top.midpoint(bottom),
            }
        };
        if !point.x.is_finite()
            || !point.y.is_finite()
            || !(0.0..width).contains(&point.x)
            || !(0.0..height).contains(&point.y)
        {
            return None;
        }
        Some((point.x / width, point.y / height))
    }
}

/// Render admission carries a deadline across asynchronous native UI-thread dispatch.
#[derive(Clone, Copy, Debug)]
pub struct ActionMarker {
    /// Validated fractions of the exact surface, from its top left.
    pub point: (f64, f64),
    /// Expiry stays fixed when a busy UI thread delays painting.
    pub expires: Instant,
}

/// One value-free action on the tracker's monotonic timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionEvent {
    /// Strictly increasing within this tracker.
    pub sequence: u64,
    /// Milliseconds since tracker creation, assigned with sequence under one lock.
    pub at_ms: u64,
    /// Owning app session, distinct from an external browser's lease.
    pub session_id: String,
    /// Input kind.
    pub kind: ActionKind,
    /// Exact browser and surface binding.
    pub source: ActionSource,
    /// Optional value-free geometry.
    pub geometry: ActionGeometry,
}

/// Bounded ephemeral fan-out. No subscribers means no retained actions; lag is never replayed as fresh feedback.
pub struct ActionTracker {
    start: Instant,
    sequence: Mutex<u64>,
    events: broadcast::Sender<ActionEvent>,
}

impl Default for ActionTracker {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            sequence: Mutex::new(0),
            events: broadcast::channel(64).0,
        }
    }
}

impl ActionTracker {
    /// Observe future actions only; a new view cannot replay an earlier tab's clicks.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ActionEvent> {
        self.events.subscribe()
    }

    /// Publish one completed agent action. Entire production call sites are behind `click-marker`.
    pub fn record(
        &self,
        session_id: String,
        kind: ActionKind,
        source: ActionSource,
        geometry: ActionGeometry,
    ) -> ActionEvent {
        let mut sequence = self.sequence.lock().unwrap_or_else(PoisonError::into_inner);
        *sequence = sequence.saturating_add(1);
        let event = ActionEvent {
            sequence: *sequence,
            at_ms: u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX),
            session_id,
            kind,
            source,
            geometry,
        };
        let _ = self.events.send(event.clone());
        event
    }

    /// Remaining display lifetime on this tracker's clock; future or expired events cannot draw.
    #[must_use]
    pub fn remaining(&self, event: &ActionEvent) -> Option<Duration> {
        let age = self
            .start
            .elapsed()
            .checked_sub(Duration::from_millis(event.at_ms))?;
        MARKER_LIFETIME
            .checked_sub(age)
            .filter(|left| !left.is_zero())
    }

    /// One overlay admission for every source. Identity and age must match before geometry is used.
    #[must_use]
    pub fn marker_for(
        &self,
        event: &ActionEvent,
        session: &str,
        source: &ActionSource,
    ) -> Option<ActionMarker> {
        if event.session_id != session
            || &event.source != source
            || source.view_id.is_none()
            || source.document_id.is_none()
            || (source.browser != SourceBrowser::BuiltIn
                && (source.profile_id.is_none() || source.browser_session_id.is_none()))
        {
            return None;
        }
        let remaining = self.remaining(event)?;
        Some(ActionMarker {
            point: event.geometry.normalized_point()?,
            expires: Instant::now() + remaining,
        })
    }
}

#[cfg(test)]
#[path = "action_tracking_tests.rs"]
mod tests;
