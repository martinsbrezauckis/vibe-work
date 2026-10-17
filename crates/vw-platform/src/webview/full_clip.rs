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

#[cfg(test)]
mod tests {
    use super::{FullClip, full_clip};

    /// An image side in pixels from its exact value (Chromium rounds half away from zero).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn pixels(v: f64) -> u32 {
        v.round() as u32
    }

    /// What Chromium makes of `clip` for a webview at page zoom `zoom` on a screen of device pixel ratio `dpr`
    /// (see the module comment): the image size and the image pixels one CSS pixel is drawn as.
    fn rendered(clip: FullClip, zoom: f64, dpr: f64) -> ((u32, u32), f64) {
        (
            (
                pixels(clip.width * dpr * clip.scale),
                pixels(clip.height * dpr * clip.scale),
            ),
            zoom * clip.scale * dpr,
        )
    }

    /// The clip taken before the fix: the document's CSS size at scale 1, whatever the zoom.
    fn unscaled(css: (u32, u32)) -> FullClip {
        FullClip {
            width: f64::from(css.0),
            height: f64::from(css.1),
            scale: 1.0,
        }
    }

    /// A page of 1280 x 974 CSS pixels in a preview zoomed to 0.6 on a 1.5 ratio screen is an image of
    /// 1920 x 1461 pixels with one CSS pixel drawn as 1.5 of them. The old clip gave the same image size with the
    /// page at 0.9 of its size.
    #[test]
    fn zoomed_preview_is_captured_at_the_pages_real_scale() {
        let css = (1280, 974);
        let ((w, h), per_css) = rendered(full_clip(css, 0.6), 0.6, 1.5);
        assert_eq!((w, h), (1920, 1461));
        assert!((per_css - 1.5).abs() < 1e-9, "{per_css}");
        let (_, old) = rendered(unscaled(css), 0.6, 1.5);
        assert!(
            (old - 0.9).abs() < 1e-9,
            "the old clip drew the page at {old}"
        );
    }

    /// Every zoom from Chromium's browser zoom limits (0.25 to 5; `WebView2` brings a zoom outside its range into
    /// it), every ratio Windows scales a screen to: the image is the page's CSS size times the ratio and the page
    /// fills it. The sizes
    /// are multiples of 4, so no side times a ratio ends in .5 (where one pixel either way is the same capture).
    #[test]
    fn image_is_css_size_times_ratio_for_any_zoom_and_ratio() {
        for zoom in [0.25, 0.2995, 0.5, 0.6, 0.59375, 0.9, 1.0, 1.25, 2.0, 5.0] {
            for dpr in [1.0, 1.25, 1.5, 1.75, 2.0, 3.0] {
                for css in [(1280, 976), (392, 5000), (4096, 16_384), (4, 4)] {
                    let ((w, h), per_css) = rendered(full_clip(css, zoom), zoom, dpr);
                    let want = |v: u32| pixels(f64::from(v) * dpr);
                    assert_eq!(
                        (w, h),
                        (want(css.0), want(css.1)),
                        "{css:?} zoom {zoom} ratio {dpr}"
                    );
                    assert!(
                        (per_css - dpr).abs() < 1e-9,
                        "{css:?} zoom {zoom} ratio {dpr}: {per_css}"
                    );
                }
            }
        }
    }

    /// Without a zoom the clip is the old one, so a `fit` preview (and every web tab) is captured exactly as before.
    #[test]
    fn no_zoom_leaves_the_clip_unchanged() {
        assert_eq!(full_clip((1280, 974), 1.0), unscaled((1280, 974)));
    }

    /// The request text is valid JSON with the whole-document options and the corrected clip (what `capture_png`
    /// sends to `WebView2`).
    #[test]
    fn params_are_the_capture_request_with_the_corrected_clip() {
        let text = full_clip((1280, 974), 0.5).params();
        let v: serde_json::Value = serde_json::from_str(&text).expect(&text);
        assert_eq!(v["format"], "png");
        assert_eq!(v["fromSurface"], true);
        assert_eq!(v["captureBeyondViewport"], true);
        let clip = &v["clip"];
        assert_eq!(
            (clip["x"].as_f64(), clip["y"].as_f64()),
            (Some(0.0), Some(0.0))
        );
        assert_eq!(clip["width"].as_f64(), Some(640.0));
        assert_eq!(clip["height"].as_f64(), Some(487.0));
        assert_eq!(clip["scale"].as_f64(), Some(2.0));
    }

    /// A zoom the controller could not give (error, zero, negative, NaN, infinity) counts as none; the clip stays
    /// finite and positive.
    #[test]
    fn an_unusable_zoom_counts_as_none() {
        for zoom in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(full_clip((800, 600), zoom), unscaled((800, 600)), "{zoom}");
        }
    }
}
