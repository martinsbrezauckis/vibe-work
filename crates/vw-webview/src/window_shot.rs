//! Whole-window screenshots of the app's own windows, for the Debug API's `/screenshot`.
//!
//! "only the app's own windows, never the screen or other apps": the main window exactly as
//! shown (overlays, menus, toasts and the child webviews: previews, the Project tab's viewer), plus, as separate
//! images, native dialogs of Vibe Work when the OS allows it. Adapted from ShellX
//! `src-tauri/src/debug_api_screenshot.rs` @ e38c2b5c (Windows `PrintWindow`); Linux and macOS compose the
//! window from its webviews' own snapshots instead of reading screen pixels, so a window behind other windows is
//! captured correctly (ShellX's X11 read refused an obscured window).
//!
//! Per OS (what is and is not captured is in `docs/public/DEBUG_API.md`):
//! - Windows: `PrintWindow` of the main window's client area (all child `WebView2` windows included), converted
//!   from the display's ICC profile to sRGB (found `WebView2` renders in the display's colour space);
//!   native dialogs (the confirmation box, the folder picker) are this process's other visible top-level
//!   windows, each captured with its frame.
//! - Linux: every mapped webview of the main window is snapshotted by `WebKitGTK` and pasted at its place in the
//!   main webview (GTK coordinates); GTK dialogs of this process (confirmations, the folder picker) are drawn
//!   offscreen with `gtk_widget_draw`.
//! - macOS: every visible `WKWebView` is snapshotted and pasted at its place in the content view; native panels
//!   are not captured.
//!
//! Depends on: tauri, vw-preview (decode, compose, encode, ICC), vw-platform (`window_capture`), `platform`
//! (`with_native`, `screenshot`), gtk/cairo on Linux. Called by: the app (`window_api.rs`).
//! Invariants:
//! - Pixels come only from this app's windows and webviews; nothing reads the screen.
//! - no picture of the main window while one of its previews holds a vault-filled password
//!   (`vw_preview::shots_protected`).
//! - Every wait for the UI thread is bounded (`platform::with_native`, the main-thread timeouts below).

#[cfg(not(windows))]
use std::time::Duration;

use tauri::{Manager, Runtime, Window, Wry};
use vw_preview::Rgba;

/// Longest wait for one snapshot or one UI-thread answer.
#[cfg(not(windows))]
const LIMIT: Duration = Duration::from_secs(8);

/// One of the app's own windows (the main window is `main`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnWindow {
    /// Id for [`capture`].
    pub id: String,
    /// `main` or `dialog`.
    pub kind: &'static str,
    /// Title.
    pub title: String,
    /// Size in physical pixels.
    pub width: u32,
    /// Size in physical pixels.
    pub height: u32,
}

/// The app's own windows: the main window first, then native dialogs this OS lets the app see.
///
/// # Errors
/// A text reason when the main window is gone.
pub async fn windows(window: &Window<Wry>) -> Result<Vec<OwnWindow>, String> {
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let mut out = vec![OwnWindow {
        id: "main".to_owned(),
        kind: "main",
        title: window.title().unwrap_or_default(),
        width: size.width,
        height: size.height,
    }];
    out.extend(dialogs(window).await);
    Ok(out)
}

/// A PNG (sRGB) of window `id` (`main` or a dialog id from [`windows`]).
///
/// # Errors
/// A text reason (unknown id, a failed capture).
pub async fn capture(window: &Window<Wry>, id: &str) -> Result<Vec<u8>, String> {
    // a preview a vault password was filled into is never pictured (ShellX's protected values); the window
    // shows that preview as a child webview, so the whole window is refused until the page loads another document.
    if id == "main"
        && window
            .webviews()
            .iter()
            .any(|w| vw_preview::shots_protected(w.label()))
    {
        return Err(vw_preview::SHOTS_BLOCKED.to_owned());
    }
    let image = if id == "main" {
        main_image(window).await?
    } else {
        dialog_image(window, id).await?
    };
    vw_preview::encode(&image)
}

/// Decodes an engine PNG (ICC-tagged ones are converted to sRGB by `decode`).
#[cfg(not(windows))]
fn decoded(png: &[u8]) -> Result<Rgba, String> {
    vw_preview::decode(png)
}

// ---- Windows -------------------------------------------------------------------------------------------------

#[cfg(windows)]
fn raw_to_rgba(raw: vw_platform::window_capture::RawCapture) -> Rgba {
    let pixels = match raw.icc {
        Some(profile) => vw_preview::to_srgb(&profile, raw.rgba),
        None => raw.rgba,
    };
    Rgba {
        width: raw.width,
        height: raw.height,
        pixels,
    }
}

#[cfg(windows)]
async fn main_image(window: &Window<Wry>) -> Result<Rgba, String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    let raw =
        tokio::task::spawn_blocking(move || vw_platform::window_capture::print_window(hwnd, true))
            .await
            .map_err(|e| e.to_string())??;
    Ok(raw_to_rgba(raw))
}

#[cfg(windows)]
async fn dialogs(window: &Window<Wry>) -> Vec<OwnWindow> {
    let main = window.hwnd().map_or(0, |h| h.0 as isize);
    tokio::task::spawn_blocking(move || {
        vw_platform::window_capture::own_windows()
            .into_iter()
            // Only real dialogs: owned by the main window (Vibe Work parents every dialog to it) with a title, or
            // a standard dialog box. Helper windows of the event loop or WebView2 are never listed (the first
            // Windows 11 run listed two untitled helpers; closing them closed the main window).
            .filter(|w| {
                w.hwnd != main && !w.title.is_empty() && (w.owner == main || w.class == "#32770")
            })
            .map(|w| OwnWindow {
                id: format!("win-{:x}", w.hwnd),
                kind: "dialog",
                title: w.title,
                width: 0,
                height: 0,
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

#[cfg(windows)]
async fn dialog_image(window: &Window<Wry>, id: &str) -> Result<Rgba, String> {
    // Only a dialog that is listed right now: never the main window under a dialog id (its capture goes through
    // `main_image`, which blanks secret fields first; this path takes raw pixels) and never an unlisted helper
    // window of this process. `print_window` still refuses any window of another process.
    if !dialogs(window).await.iter().any(|w| w.id == id) {
        return Err("no such dialog (it may have closed)".to_owned());
    }
    let hwnd = id
        .strip_prefix("win-")
        .and_then(|h| isize::from_str_radix(h, 16).ok())
        .ok_or("unknown window id")?;
    let raw =
        tokio::task::spawn_blocking(move || vw_platform::window_capture::print_window(hwnd, false))
            .await
            .map_err(|e| e.to_string())??;
    Ok(raw_to_rgba(raw))
}

// ---- Linux and macOS: compose the webviews ------------------------------------------------------------------

/// Where one webview sits, in device pixels relative to the main webview's top-left.
#[cfg(not(windows))]
#[derive(Clone, Copy, Debug)]
struct Place {
    visible: bool,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}

#[cfg(target_os = "linux")]
async fn place_of(webview: &tauri::Webview<Wry>) -> Result<Place, String> {
    crate::platform::with_native(webview, "locating a webview", LIMIT, |native, reply| {
        use gtk::prelude::*;
        let view = native.inner();
        let top = view.toplevel();
        let origin = top
            .as_ref()
            .and_then(|t| view.translate_coordinates(t, 0, 0))
            .unwrap_or((0, 0));
        let alloc = view.allocation();
        let _ = reply.send(Ok(Place {
            visible: view.is_visible()
                && view.is_mapped()
                && alloc.width() > 1
                && alloc.height() > 1,
            x: f64::from(origin.0),
            y: f64::from(origin.1),
            width: f64::from(alloc.width()),
            height: f64::from(alloc.height()),
            scale: f64::from(view.scale_factor()),
        }));
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
async fn place_of(webview: &tauri::Webview<Wry>) -> Result<Place, String> {
    crate::platform::with_native(webview, "locating a webview", LIMIT, |native, reply| {
        let place = vw_platform::window_capture::view_place(native.inner()).map(|p| Place {
            visible: !p.hidden && p.width > 1.0 && p.height > 1.0,
            x: p.x,
            y: p.y,
            width: p.width,
            height: p.height,
            scale: p.scale,
        });
        let _ = reply.send(place.ok_or_else(|| "the webview is not in a window".to_owned()));
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn to_px(v: f64) -> i64 {
    // Window coordinates and sizes in device pixels: far inside i64.
    #[allow(clippy::cast_possible_truncation)]
    let px = v.round() as i64;
    px
}

#[cfg(not(windows))]
async fn main_image(window: &Window<Wry>) -> Result<Rgba, String> {
    let webviews = window.webviews();
    let main = webviews
        .iter()
        .find(|w| w.label() == window.label())
        .ok_or("the main webview is not mounted")?;
    let base = place_of(main).await?;
    let main_png = crate::platform::screenshot(main, LIMIT, None)
        .await
        .map_err(|e| e.to_string())?;
    let first = decoded(&main_png)?;
    let scale = base.scale.max(1.0);
    let width = u32::try_from(to_px(base.width * scale).max(1)).unwrap_or(1);
    let height = u32::try_from(to_px(base.height * scale).max(1)).unwrap_or(1);
    let mut canvas = vw_preview::canvas(width, height, [0, 0, 0, 255]);
    vw_preview::paste(
        &mut canvas,
        &first,
        vw_preview::PixelRect {
            x: 0,
            y: 0,
            width,
            height,
        },
    );
    for child in webviews.iter().filter(|w| w.label() != window.label()) {
        let Ok(place) = place_of(child).await else {
            continue;
        };
        if !place.visible {
            continue;
        }
        let Ok(png) = crate::platform::screenshot(child, LIMIT, None).await else {
            tracing::warn!(
                webview = child.label(),
                "window screenshot: a child webview gave no snapshot"
            );
            continue;
        };
        let Ok(part) = decoded(&png) else { continue };
        let rect = vw_preview::PixelRect {
            x: to_px((place.x - base.x) * scale),
            y: to_px((place.y - base.y) * scale),
            width: u32::try_from(to_px(place.width * scale).max(1)).unwrap_or(1),
            height: u32::try_from(to_px(place.height * scale).max(1)).unwrap_or(1),
        };
        vw_preview::paste(&mut canvas, &part, rect);
    }
    Ok(canvas)
}

// ---- Linux dialogs: GTK windows of this process ---------------------------------------------------------------

#[cfg(target_os = "linux")]
async fn on_main_thread<T: Send + 'static>(
    window: &Window<Wry>,
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let _ = tx.send(job());
        })
        .map_err(|e| e.to_string())?;
    tokio::time::timeout(LIMIT, rx)
        .await
        .map_err(|_| "the UI thread did not answer".to_owned())?
        .map_err(|_| "the UI thread dropped the request".to_owned())
}

/// GTK top-level windows of this process that are shown and are not the main window, with a stable id (the
/// widget's address while it lives).
#[cfg(target_os = "linux")]
fn gtk_dialogs(main_title: &str) -> Vec<(String, gtk::Window)> {
    use glib::prelude::*;
    use gtk::prelude::*;
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Window>().ok())
        .filter(|w| w.is_visible() && w.window_type() == gtk::WindowType::Toplevel)
        .filter(|w| {
            // The main window is the one holding the main webview; dialogs are transient for it.
            w.transient_for().is_some() || w.title().is_some_and(|t| t != main_title)
        })
        .map(|w| (format!("gtk-{:x}", w.as_ptr() as usize), w))
        .collect()
}

#[cfg(target_os = "linux")]
async fn dialogs(window: &Window<Wry>) -> Vec<OwnWindow> {
    let main_title = window.title().unwrap_or_default();
    on_main_thread(window, move || {
        use gtk::prelude::*;
        gtk_dialogs(&main_title)
            .into_iter()
            .map(|(id, w)| {
                let alloc = w.allocation();
                let scale = w.scale_factor();
                OwnWindow {
                    id,
                    kind: "dialog",
                    title: w.title().map(|t| t.to_string()).unwrap_or_default(),
                    width: u32::try_from(alloc.width() * scale).unwrap_or(0),
                    height: u32::try_from(alloc.height() * scale).unwrap_or(0),
                }
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

#[cfg(target_os = "linux")]
async fn dialog_image(window: &Window<Wry>, id: &str) -> Result<Rgba, String> {
    let main_title = window.title().unwrap_or_default();
    let wanted = id.to_owned();
    let png = on_main_thread(window, move || -> Result<Vec<u8>, String> {
        use gtk::prelude::*;
        let (_, dialog) = gtk_dialogs(&main_title)
            .into_iter()
            .find(|(id, _)| *id == wanted)
            .ok_or("no such dialog (it may have closed)")?;
        let alloc = dialog.allocation();
        let scale = dialog.scale_factor();
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            alloc.width() * scale,
            alloc.height() * scale,
        )
        .map_err(|e| e.to_string())?;
        {
            let cr = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
            cr.scale(f64::from(scale), f64::from(scale));
            dialog.draw(&cr);
        }
        let mut png = Vec::new();
        surface.write_to_png(&mut png).map_err(|e| e.to_string())?;
        Ok(png)
    })
    .await??;
    decoded(&png)
}

// ---- macOS dialogs: not captured --------------------------------------------------------------------------------

// The Linux and Windows versions wait for the UI thread; here nothing waits, so these answer an already-finished
// future with the same signature (block MCL: macOS clippy `unused_async`), and the shared callers `.await` them as
// on the other platforms.

#[cfg(target_os = "macos")]
fn dialogs(_window: &Window<Wry>) -> std::future::Ready<Vec<OwnWindow>> {
    // Native panels (NSAlert, NSOpenPanel) are not listed: reading their pixels needs the screen-capture
    // permission, which the app does not ask for (only the app's own windows, never the screen).
    std::future::ready(Vec::new())
}

#[cfg(target_os = "macos")]
fn dialog_image(_window: &Window<Wry>, _id: &str) -> std::future::Ready<Result<Rgba, String>> {
    std::future::ready(Err("native dialogs are not captured on macOS".to_owned()))
}

/// The main window of `app` (its label), for the app crate.
#[must_use]
pub fn main_window<R: Runtime>(app: &tauri::AppHandle<R>, label: &str) -> Option<Window<R>> {
    app.get_window(label)
}
