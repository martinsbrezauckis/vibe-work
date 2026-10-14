//! Native mouse delivery to a single already-mounted preview webview. The preview engine calls these functions on
//! Tauri's UI thread after the browser permission and live target checks. Each path checks its own foreground window,
//! visible native surface and current geometry before sending only to that surface. Synthetic native input has no
//! guaranteed DOM `isTrusted` value; native fixture tests must establish that per engine.
//!
//! Depends on: WebView2/Win32, AppKit/WKWebView, or GTK/GDK/WebKitGTK. Called by: vw-webview `native_click`.

use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::sync::atomic::AtomicBool;
use tokio::sync::oneshot;

/// Answer channel for a checked click. Dropping its receiver (Stop or timeout) cancels pending dispatch.
pub type ClickReply = oneshot::Sender<Result<(), String>>;

/// A host-owned permission recheck, run on the UI thread immediately before native input.
pub type ClickPermission = Option<Arc<dyn Fn() -> bool + Send + Sync>>;

/// Shared guard: only an actual JavaScript boolean `true` admits native input.
#[must_use]
pub fn validation_is_true(json: &str) -> bool {
    serde_json::from_str::<bool>(json).is_ok_and(|value| value)
}

/// Validates one CSS-viewport click and returns its normalized position, strictly inside the viewport.
///
/// # Errors
/// Returns an error for non-finite, zero-size or out-of-bounds geometry.
pub fn normalized(point: (f64, f64), viewport: (f64, f64)) -> Result<(f64, f64), String> {
    let (x, y) = point;
    let (width, height) = viewport;
    if ![x, y, width, height].into_iter().all(f64::is_finite)
        || width <= 0.0
        || height <= 0.0
        || x < 0.0
        || y < 0.0
        || x >= width
        || y >= height
    {
        return Err("native click point is outside the live viewport".to_owned());
    }
    Ok((x / width, y / height))
}

/// Maps a validated viewport fraction onto a live native widget's client size. Windows uses physical client pixels,
/// while macOS and GTK use their native view coordinates; this absorbs current page zoom and OS scale.
///
/// # Errors
/// Refuses a non-finite or empty native widget, or a fraction outside its viewport.
pub fn map_point(fraction: (f64, f64), size: (f64, f64)) -> Result<(f64, f64), String> {
    if ![fraction.0, fraction.1, size.0, size.1]
        .into_iter()
        .all(f64::is_finite)
        || size.0 < 1.0
        || size.1 < 1.0
        || !(0.0..1.0).contains(&fraction.0)
        || !(0.0..1.0).contains(&fraction.1)
    {
        return Err("native preview has invalid click geometry".to_owned());
    }
    Ok((fraction.0 * size.0, fraction.1 * size.1))
}

#[cfg(target_os = "linux")]
mod linux {
    use gdk::prelude::*;
    use glib::translate::ToGlibPtr;
    use gtk::prelude::*;
    use javascriptcore::ValueExt;
    use std::sync::atomic::Ordering;
    use webkit2gtk::WebViewExt;

    use super::{Arc, AtomicBool, ClickReply};

    /// Sends a press and release into the mapped `WebKitGTK` widget via GTK's event path.
    ///
    /// # Errors
    /// Refuses inactive windows, hidden widgets and invalid native geometry.
    pub fn click(
        view: &webkit2gtk::WebView,
        point: (f64, f64),
        sent: &AtomicBool,
    ) -> Result<(), String> {
        let widget: &gtk::Widget = view.upcast_ref();
        if !widget.is_mapped()
            || !widget.is_visible()
            || !widget.is_child_visible()
            || !widget.has_window()
        {
            return Err("native preview is not visible".to_owned());
        }
        let top = widget
            .toplevel()
            .ok_or("native preview has no app window")?
            .downcast::<gtk::Window>()
            .map_err(|_| "native preview has no app window")?;
        if !top.is_active() {
            return Err("native preview window is not active".to_owned());
        }
        let window = widget.window().ok_or("native preview has no GDK window")?;
        if !window.is_viewable() {
            return Err("native preview GDK window is not viewable".to_owned());
        }
        let allocation = widget.allocation();
        if window.width() != allocation.width() || window.height() != allocation.height() {
            return Err("native preview allocation is stale".to_owned());
        }
        let width = f64::from(window.width());
        let height = f64::from(window.height());
        if width < 1.0 || height < 1.0 {
            return Err("native preview has empty bounds".to_owned());
        }
        let (x, y) = super::map_point(point, (width, height))?;
        let (_, root_x, root_y) = window.origin();
        let device = window
            .display()
            .default_seat()
            .and_then(|seat| seat.pointer())
            .ok_or("native pointer device is unavailable")?;
        let (rx, ry) = (f64::from(root_x) + x, f64::from(root_y) + y);
        move_pointer(&window, &device, x, y, rx, ry);
        sent.store(true, Ordering::Release);
        dispatch(&window, &device, gdk::EventType::ButtonPress, x, y, rx, ry);
        dispatch(
            &window,
            &device,
            gdk::EventType::ButtonRelease,
            x,
            y,
            rx,
            ry,
        );
        Ok(())
    }

    /// Evaluates the final page guard and clicks in its GTK callback, without another queue hop.
    pub fn click_checked(
        view: &webkit2gtk::WebView,
        point: (f64, f64),
        script: &str,
        sent: Arc<AtomicBool>,
        allowed: super::ClickPermission,
        reply: ClickReply,
    ) {
        let view = view.clone();
        view.clone().evaluate_javascript(
            script,
            None,
            None,
            None::<&webkit2gtk::gio::Cancellable>,
            move |answer| {
                if reply.is_closed() {
                    return;
                }
                let result = match answer {
                    Ok(value) if value.is_boolean() && value.to_boolean() => {
                        if reply.is_closed() {
                            return;
                        }
                        if allowed.as_ref().is_some_and(|allowed| !allowed()) {
                            Err("Not run: browser permission changed before native input"
                                .to_owned())
                        } else {
                            click(&view, point, &sent)
                        }
                    }
                    Ok(_) => Err("Not run: the page target changed before native input".to_owned()),
                    Err(error) => Err(format!("Not run: final page validation failed: {error}")),
                };
                let _ = reply.send(result);
            },
        );
    }

    fn move_pointer(window: &gdk::Window, device: &gdk::Device, x: f64, y: f64, rx: f64, ry: f64) {
        let mut event = gdk::Event::new(gdk::EventType::MotionNotify);
        // SAFETY: GDK allocated the motion event on GTK's UI thread. The transferred GObject references live until
        // GTK finishes dispatching the event synchronously and the event drops.
        unsafe {
            let event_ptr: *mut gdk::ffi::GdkEvent = event.to_glib_none().0;
            let motion = &mut (*event_ptr).motion;
            motion.window = window.to_glib_none().0;
            glib::gobject_ffi::g_object_ref(motion.window.cast());
            motion.device = device.to_glib_none().0;
            glib::gobject_ffi::g_object_ref(motion.device.cast());
            motion.send_event = 1;
            motion.x = x;
            motion.y = y;
            motion.x_root = rx;
            motion.y_root = ry;
            motion.time = 0; // GDK_CURRENT_TIME
        }
        gtk::main_do_event(&mut event);
    }

    fn dispatch(
        window: &gdk::Window,
        device: &gdk::Device,
        kind: gdk::EventType,
        x: f64,
        y: f64,
        root_x: f64,
        root_y: f64,
    ) {
        let mut event = gdk::Event::new(kind);
        // SAFETY: GTK created this GdkEventButton on the UI thread. The GObject refs below are transferred to the
        // event, which releases them when it is dropped. The event is synchronously delivered before that drop.
        unsafe {
            let event_ptr: *mut gdk::ffi::GdkEvent = event.to_glib_none().0;
            let button = &mut (*event_ptr).button;
            button.window = window.to_glib_none().0;
            glib::gobject_ffi::g_object_ref(button.window.cast());
            button.device = device.to_glib_none().0;
            glib::gobject_ffi::g_object_ref(button.device.cast());
            button.send_event = 1;
            button.button = 1;
            button.x = x;
            button.y = y;
            button.x_root = root_x;
            button.y_root = root_y;
            button.state = if kind == gdk::EventType::ButtonRelease {
                gdk::ffi::GDK_BUTTON1_MASK
            } else {
                0
            };
            button.time = 0; // GDK_CURRENT_TIME
        }
        // GTK explicitly recommends main_do_event over Widget.event for synthetic input.
        gtk::main_do_event(&mut event);
    }
}

#[cfg(target_os = "linux")]
pub use linux::click as linux_click;
#[cfg(target_os = "linux")]
pub use linux::click_checked as linux_click_checked;

#[cfg(windows)]
mod windows_click;
#[cfg(windows)]
pub use windows_click::click as windows_click;
#[cfg(windows)]
pub use windows_click::click_checked as windows_click_checked;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::click as macos_click;
#[cfg(target_os = "macos")]
pub use macos::click_checked as macos_click_checked;
