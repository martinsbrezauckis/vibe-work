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

#[cfg(any(target_os = "macos", test))]
mod macos_click_plan;
#[cfg(any(target_os = "macos", test))]
mod macos_wheel_plan;
#[cfg(any(windows, test))]
mod windows_sequence_plan;

/// Answer channel for a checked click. Dropping its receiver (Stop or timeout) cancels pending dispatch.
pub type ClickReply = oneshot::Sender<Result<(), String>>;

/// A host-owned permission recheck, run on the UI thread immediately before native input.
pub type ClickPermission = Option<Arc<dyn Fn() -> bool + Send + Sync>>;

/// Every native input adapter's one refusal for a window that is not in front: the app is not the active or
/// foreground one, its window is not key, or the window or the browser view is hidden (minimized, hidden to the
/// tray, pane not shown). The engine maps exactly this outcome to its "bring the window forward" answer; every other
/// adapter refusal keeps its own text. Vibe Work never brings its window forward for native input.
pub const NOT_IN_FRONT: &str = "the browser window is not in front";

/// Refusal while a native menu is open: the menu can undo a scripted selection when the person dismisses it.
/// The engine passes this to the agent as it is.
pub const MENU_OPEN: &str = "a menu or dropdown list is open in Vibe Work's window and only the user can use it, so nothing was sent. Ask the user to close the menu, then try again";

/// The in-front check shared by the click, wheel and keyboard adapters of every OS. `shown`: the browser view and
/// its window are visible (not hidden, minimized or in the tray). `front`: that window is the active, foreground or
/// key window.
///
/// # Errors
/// [`NOT_IN_FRONT`] when either is false.
pub fn require_front(shown: bool, front: bool) -> Result<(), String> {
    if shown && front {
        Ok(())
    } else {
        Err(NOT_IN_FRONT.to_owned())
    }
}

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
    use std::sync::atomic::Ordering;

    use super::AtomicBool;

    /// Sends a press and release into the mapped `WebKitGTK` widget via GTK's event path.
    ///
    /// # Errors
    /// [`super::NOT_IN_FRONT`] for an inactive or hidden window or widget; refuses invalid native geometry.
    pub fn click(
        view: &webkit2gtk::WebView,
        point: (f64, f64),
        sent: &AtomicBool,
    ) -> Result<(), String> {
        click_options(view, point, sent, (0, 1, 0, None))
    }

    /// Sends a bounded click variant: DOM button, click count and Alt/Control/Meta/Shift bits.
    ///
    /// # Errors
    /// The native view is unavailable, hidden, inactive or has invalid geometry/options.
    pub fn click_options(
        view: &webkit2gtk::WebView,
        point: (f64, f64),
        sent: &AtomicBool,
        options: (u8, u8, u8, Option<(i32, i32)>),
    ) -> Result<(), String> {
        let (button, count, modifiers, wheel) = options;
        let options = (button, count, modifiers);
        if wheel.is_some_and(|(x, y)| {
            count != 0
                || modifiers != 0
                || button != 0
                || !(-20_000..=20_000).contains(&x)
                || !(-20_000..=20_000).contains(&y)
                || (x == 0 && y == 0)
        }) {
            return Err("invalid native wheel options".into());
        }
        if options != (0, 0, 0) && (button > 2 || !(1..=3).contains(&count) || modifiers > 15) {
            return Err("invalid native click options".into());
        }
        if super::linux_menu_open() {
            return Err(super::MENU_OPEN.to_owned());
        }
        let widget: &gtk::Widget = view.upcast_ref();
        let top = widget
            .toplevel()
            .ok_or("native preview has no app window")?
            .downcast::<gtk::Window>()
            .map_err(|_| "native preview has no app window")?;
        super::require_front(
            widget.is_mapped()
                && widget.is_visible()
                && widget.is_child_visible()
                && widget.has_window(),
            top.is_active(),
        )?;
        let window = widget.window().ok_or("native preview has no GDK window")?;
        // Not viewable: an ancestor is unmapped (the window is hidden or minimized).
        super::require_front(window.is_viewable(), true)?;
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
        // Marked as our own so the person's pointer watch leaves it out.
        super::person::own_input(|| {
            sent.store(true, Ordering::Release);
            move_pointer(&window, &device, x, y, rx, ry);
            if let Some(delta) = wheel {
                wheel_event(&window, &device, (x, y, rx, ry), delta);
            }
            for n in 1..=count {
                let kind = match n {
                    2 => gdk::EventType::DoubleButtonPress,
                    3 => gdk::EventType::TripleButtonPress,
                    _ => gdk::EventType::ButtonPress,
                };
                dispatch(&window, &device, kind, (x, y, rx, ry), options);
                dispatch(
                    &window,
                    &device,
                    gdk::EventType::ButtonRelease,
                    (x, y, rx, ry),
                    options,
                );
            }
        });
        Ok(())
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

    fn wheel_event(
        window: &gdk::Window,
        device: &gdk::Device,
        point: (f64, f64, f64, f64),
        delta: (i32, i32),
    ) {
        let mut event = gdk::Event::new(gdk::EventType::Scroll);
        // SAFETY: GDK allocated this scroll event on the UI thread; retained window/device refs belong to it.
        unsafe {
            let event_ptr: *mut gdk::ffi::GdkEvent = event.to_glib_none().0;
            let scroll = &mut (*event_ptr).scroll;
            scroll.window = window.to_glib_none().0;
            glib::gobject_ffi::g_object_ref(scroll.window.cast());
            scroll.device = device.to_glib_none().0;
            glib::gobject_ffi::g_object_ref(scroll.device.cast());
            scroll.send_event = 1;
            scroll.x = point.0;
            scroll.y = point.1;
            scroll.x_root = point.2;
            scroll.y_root = point.3;
            scroll.direction = gdk::ffi::GDK_SCROLL_SMOOTH;
            // WebKitGTK scales mouse-wheel units by floor(view extent^(2/3)); touchpads use 40.
            // Reverse that native scaling so the page receives the requested pixel deltas.
            let step = |extent: i32| {
                if device.source() == gdk::InputSource::Mouse {
                    f64::from(extent).powf(2.0 / 3.0).floor().max(1.0)
                } else {
                    40.0
                }
            };
            scroll.delta_x = f64::from(delta.0) / step(window.width());
            scroll.delta_y = f64::from(delta.1) / step(window.height());
            scroll.time = 0;
        }
        gtk::main_do_event(&mut event);
    }

    fn dispatch(
        window: &gdk::Window,
        device: &gdk::Device,
        kind: gdk::EventType,
        position: (f64, f64, f64, f64),
        options: (u8, u8, u8),
    ) {
        let (x, y, root_x, root_y) = position;
        let (dom_button, _, modifiers) = options;
        let native_button = match dom_button {
            1 => 2,
            2 => 3,
            _ => 1,
        };
        let held = match native_button {
            2 => gdk::ffi::GDK_BUTTON2_MASK,
            3 => gdk::ffi::GDK_BUTTON3_MASK,
            _ => gdk::ffi::GDK_BUTTON1_MASK,
        };
        let state = if modifiers & 1 != 0 {
            gdk::ffi::GDK_MOD1_MASK
        } else {
            0
        } | if modifiers & 2 != 0 {
            gdk::ffi::GDK_CONTROL_MASK
        } else {
            0
        } | if modifiers & 4 != 0 {
            gdk::ffi::GDK_META_MASK | gdk::ffi::GDK_SUPER_MASK
        } else {
            0
        } | if modifiers & 8 != 0 {
            gdk::ffi::GDK_SHIFT_MASK
        } else {
            0
        };
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
            button.button = native_button;
            button.x = x;
            button.y = y;
            button.x_root = root_x;
            button.y_root = root_y;
            button.state = state
                | if kind == gdk::EventType::ButtonRelease {
                    held
                } else {
                    0
                };
            button.time = 0; // GDK_CURRENT_TIME
        }
        // GTK explicitly recommends main_do_event over Widget.event for synthetic input.
        gtk::main_do_event(&mut event);
    }
}

/// Keyboard translation for the same checked native frame path.
pub mod keyboard;
/// The person's own pointer activity on a preview webview (see `person`).
pub mod person;

#[cfg(target_os = "linux")]
pub use linux::click as linux_click;
#[cfg(target_os = "linux")]
pub use linux::click_options as linux_click_options;

#[cfg(windows)]
mod windows_click;
#[cfg(windows)]
pub use windows_click::click as windows_click;

#[cfg(target_os = "macos")]
mod macos;
/// Whether `AppKit`'s event-tracking loop is holding a native menu or gesture.
#[cfg(target_os = "macos")]
#[must_use]
pub fn macos_menu_open() -> bool {
    macos::refuse_open_menu().is_err()
}
/// Whether this process currently holds a GTK grab. `WebKitGTK`'s native select popup grabs a
/// `GtkWindow`; an ordinary background app has no grab. Call on the GTK UI thread.
#[cfg(target_os = "linux")]
#[must_use]
pub fn linux_menu_open() -> bool {
    gtk::grab_get_current().is_some()
}
#[cfg(target_os = "macos")]
pub use macos::after as macos_after;
#[cfg(target_os = "macos")]
pub use macos::click as macos_click;
#[cfg(target_os = "macos")]
pub use macos::click_options as macos_click_options;

#[cfg(windows)]
mod windows_sequence;
#[cfg(windows)]
pub use windows_sequence::click as windows_click_options;
