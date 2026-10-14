//! Where the preview webview goes and at which zoom, from the placeholder rectangle the UI reports and the
//! chosen device width (ShellX pattern: lay the page out at a true device width, scale it to fit).
//!
//! Depends on: serde. Called by: `service`, tests.
//! Invariants:
//! - Bounds are finite, at least [`MIN_SIDE`] on each side, and inside `0..=MAX_COORD`
//!   (following ShellX's `engine_bounds_rect`).
//! - A device width narrower than the rectangle is shown at its real size, centred, zoom 1; a wider one fills
//!   the rectangle with zoom = rectangle width / device width, so the page's CSS viewport is the device width.
//! - A preview that is not on screen is parked at the placement it would have if it were shown now
//!   ([`parked`]), so a background session's page lays out like the one the user sees when they open
//!   the tab, never at the engine's smallest size.

use serde::{Deserialize, Serialize};

/// Smallest side of a placed webview in logical pixels.
pub const MIN_SIDE: f64 = 16.0;
/// Largest coordinate accepted (well inside every platform's native range).
pub const MAX_COORD: f64 = 100_000.0;
/// Smallest zoom used for a wide device in a narrow panel (beyond this the page is unreadable anyway).
pub const MIN_ZOOM: f64 = 0.2;
/// The area a hidden preview is laid out in before the window has reported any preview area: a common
/// desktop viewport (1280 x 800 logical px).
pub const PARKED_AREA: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 1280.0,
    height: 800.0,
};

/// The device width a preview is laid out at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DeviceWidth {
    /// Phone: 390 CSS px.
    Phone,
    /// Tablet: 768 CSS px.
    Tablet,
    /// Desktop: 1280 CSS px.
    Desktop,
    /// The panel's own width, no scaling.
    #[default]
    Fit,
}

impl DeviceWidth {
    /// The CSS width, `None` for `Fit`.
    #[must_use]
    pub fn css_px(self) -> Option<f64> {
        match self {
            Self::Phone => Some(390.0),
            Self::Tablet => Some(768.0),
            Self::Desktop => Some(1280.0),
            Self::Fit => None,
        }
    }
}

/// A rectangle in the main window's logical pixels (CSS pixels of the main webview).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    /// Left.
    pub x: f64,
    /// Top.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Rect {
    /// Checks the rectangle (finite, in range, not too small).
    ///
    /// # Errors
    /// A text reason.
    pub fn checked(self) -> Result<Self, String> {
        let all = [self.x, self.y, self.width, self.height];
        if all.iter().any(|v| !v.is_finite()) {
            return Err("preview bounds must be finite".to_owned());
        }
        if all.iter().any(|v| *v < 0.0 || *v > MAX_COORD) {
            return Err("preview bounds are out of range".to_owned());
        }
        if self.width < MIN_SIDE || self.height < MIN_SIDE {
            return Err("preview bounds are too small".to_owned());
        }
        Ok(self)
    }
}

/// Where the webview goes and its zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Native bounds of the webview (logical pixels).
    pub bounds: Rect,
    /// Page zoom (1 = none).
    pub zoom: f64,
}

/// The placement of a preview laid out at `width` inside `area`.
///
/// # Errors
/// A text reason when `area` is not a valid rectangle.
pub fn place(area: Rect, width: DeviceWidth) -> Result<Placement, String> {
    let area = area.checked()?;
    let Some(device) = width.css_px() else {
        return Ok(Placement {
            bounds: area,
            zoom: 1.0,
        });
    };
    if device <= area.width {
        let x = area.x + ((area.width - device) / 2.0).floor();
        return Ok(Placement {
            bounds: Rect {
                x,
                width: device,
                ..area
            },
            zoom: 1.0,
        });
    }
    Ok(Placement {
        bounds: area,
        zoom: (area.width / device).max(MIN_ZOOM),
    })
}

/// The placement a preview that is NOT on screen is parked at: the one it would get if its session were
/// shown now, i.e. `place(last_area, width)` with the last preview area the window reported, or [`PARKED_AREA`] when
/// none was reported yet (or the last one is not a valid rectangle). The engine keeps the webview hidden at this
/// size, so the page's layout (media queries, `innerWidth`, element boxes) matches what the user sees when they open
/// the tab, while nothing is drawn.
#[must_use]
pub fn parked(last_area: Option<Rect>, width: DeviceWidth) -> Placement {
    last_area
        .and_then(|area| place(area, width).ok())
        .or_else(|| place(PARKED_AREA, width).ok())
        // PARKED_AREA is a valid rectangle, so `place` cannot fail on it; kept total anyway.
        .unwrap_or(Placement {
            bounds: PARKED_AREA,
            zoom: 1.0,
        })
}
