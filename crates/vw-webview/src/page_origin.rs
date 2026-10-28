//! Where the app page's coordinates are in the main window: every rectangle the UI reports for a native child
//! webview (preview, Browser page, Pages and File tab viewers) is in the app page's CSS pixels, while Tauri's
//! `set_bounds` and `add_child` position a child in the window's content view.
//!
//! Depends on: tauri (`Window::webviews`, feature `unstable`), vw-platform (`window_capture::view_place`, macOS),
//! `platform::with_native`. Called by: `platform` (place, park), `engine` (a new child's first position).
//! Invariants:
//! - Linux and Windows: the identity (no native call); only macOS moves a rectangle.
//! - macOS: read from the main webview on every use (full screen has no title bar; the title bar's height can change).

#[cfg(target_os = "macos")]
use std::time::Duration;

use tauri::Wry;
use vw_preview::EngineError;

#[cfg(target_os = "macos")]
use crate::platform::with_native;

/// Where the app page's CSS `(0, 0)` is in the window's content view, logical points from its top-left: the space
/// Tauri's `set_bounds` and `add_child` position child webviews in (wry measures `y` from the content view's top).
/// Every rectangle the UI reports is in the app page's CSS pixels, so it is moved by this before it is applied.
/// - Linux and Windows: `(0, 0)` (the app page fills the window's client area; Linux places through its own layer).
/// - macOS: below the title bar. Tauri's default title bar style makes the content view full size under an opaque
///   title bar, the app's `WKWebView` fills it from the top, and `WebKit` insets that page by the title bar
///   (`vw_platform::window_capture::viewport_top`). Unmoved, every preview was drawn the title bar's height (32 pt)
///   higher than its area, over the Browser toolbar (macOS 27, measured: content view 800 pt, `contentLayoutRect`
///   768 pt, the app page 768 CSS px). Read from the main webview each time: it changes with full
///   screen (no title bar) and with the window's title bar.
#[cfg_attr(not(target_os = "macos"), allow(clippy::unused_async))]
pub(crate) async fn css_origin(window: &tauri::Window<Wry>) -> Result<(f64, f64), EngineError> {
    #[cfg(target_os = "macos")]
    {
        let main = window
            .webviews()
            .into_iter()
            .find(|w| w.label() == window.label())
            .ok_or_else(|| EngineError::Platform("the main webview is not mounted".to_owned()))?;
        with_native(
            &main,
            "locating the app's page",
            Duration::from_secs(5),
            |native, reply| {
                let place = vw_platform::window_capture::view_place(native.inner());
                let _ = reply.send(
                    place
                        .map(|p| (p.x, p.viewport_top))
                        .ok_or_else(|| "the app's page is not in a window".to_owned()),
                );
            },
        )
        .await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Ok((0.0, 0.0))
    }
}

/// `bounds` (the app page's CSS pixels) in the content view's coordinates, given the page's [`css_origin`].
#[must_use]
pub(crate) fn in_window(bounds: vw_preview::Rect, (x, y): (f64, f64)) -> vw_preview::Rect {
    vw_preview::Rect {
        x: bounds.x + x,
        y: bounds.y + y,
        ..bounds
    }
}

#[cfg(test)]
mod tests {
    use super::in_window;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> vw_preview::Rect {
        vw_preview::Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_page_below_a_title_bar_moves_the_preview_down_by_it() {
        // macOS: the Browser's page area at CSS y 296, the app page 32 pt below the content view's top.
        assert_eq!(
            in_window(rect(780.0, 296.0, 500.0, 538.0), (0.0, 32.0)),
            rect(780.0, 328.0, 500.0, 538.0)
        );
    }

    #[test]
    fn a_page_at_the_window_top_keeps_the_reported_bounds() {
        // Linux and Windows (`css_origin` is (0, 0)), and macOS without an opaque full-size title bar.
        let area = rect(780.0, 296.0, 500.0, 538.0);
        assert_eq!(in_window(area, (0.0, 0.0)), area);
    }
}
