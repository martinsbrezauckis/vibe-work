//! The `DevTools` clip of a one-render full-page capture, corrected for the webview's page zoom.
//!
//! `Page.captureScreenshot` with `captureBeyondViewport` and a `clip` renders the page through a viewport
//! emulation: the clip's width and height are in the zoomed page's own pixels (what the page measures after the
//! webview's `ZoomFactor`), the image is `clip x scale x device pixel ratio` pixels, and one CSS pixel is drawn
//! `zoom x scale x device pixel ratio` image pixels wide (Chromium `content/browser/devtools/protocol/page_handler.cc`,
//! `CaptureScreenshot`: `viewport_scale = clip.scale`, `requested_image_size = round(clip x scale x DSF)`; the page
//! zoom is the layout's own and is not part of that arithmetic). A preview laid out at a device width wider than its
//! panel is shown at zoom = panel / device width (`vw-preview` `layout::place`), so the old clip of the document's
//! CSS size at scale 1 produced an image of that many pixels with the page drawn at `zoom` of its size in the
//! upper-left corner (installed Windows build: a 1280 x 973 picture with the page at 59%).
//!
//! The corrected clip asks for the same region in the zoomed page's pixels (`css x zoom`) at scale `1 / zoom`, so the
//! page is drawn one device pixel per CSS pixel and the image is `css x device pixel ratio`, whatever the zoom. Nothing
//! about the webview changes (no zoom reset, no resize), so there is nothing to restore after a Stop or a timeout and
//! the page keeps the CSS viewport (the device width) it was laid out at.
//!
//! Depends on: nothing. Called by: `webview::capture_png` (Windows), tests.
//! Invariants: total (any zoom, also NaN, gives a finite clip); the zoom the webview really has is read from the
//! controller by the caller, never assumed.

/// The `clip` of a full-page `Page.captureScreenshot` (its `x` and `y` are 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FullClip {
    /// Width in the zoomed page's pixels.
    pub width: f64,
    /// Height in the zoomed page's pixels.
    pub height: f64,
    /// Page scale of the render.
    pub scale: f64,
}

impl FullClip {
    /// The `Page.captureScreenshot` parameters (JSON text) that take the whole document through this clip.
    #[must_use]
    pub fn params(self) -> String {
        let Self {
            width,
            height,
            scale,
        } = self;
        format!(
            r#"{{"format":"png","fromSurface":true,"captureBeyondViewport":true,"clip":{{"x":0,"y":0,"width":{width},"height":{height},"scale":{scale}}}}}"#
        )
    }
}

/// The clip that captures a document of `css` (width, height) CSS pixels at one device pixel (times the screen's
/// ratio) per CSS pixel while the webview's page zoom is `zoom` (1 = none). A zoom that is not a positive finite
/// number counts as 1.
#[must_use]
pub fn full_clip(css: (u32, u32), zoom: f64) -> FullClip {
    let zoom = if zoom.is_finite() && zoom > 0.0 {
        zoom
    } else {
        1.0
    };
    FullClip {
        width: f64::from(css.0) * zoom,
        height: f64::from(css.1) * zoom,
        scale: 1.0 / zoom,
    }
}
