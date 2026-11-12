//! Host-side watch of the person's own pointer presses and releases on one preview webview, so checked native
//! clicks can wait while the person is clicking and never take the person's click for the agent's. Page script
//! cannot reach these native events, and the input this module's caller submits itself is left out.
//!
//! Linux: the widget's GTK event signal (real window-system input; our own input is marked while it is sent).
//! macOS: an application-local `NSEvent` monitor, which sees events the application dispatches; our own input is
//! delivered straight to the view and never passes through it.
//! Windows: a low-level mouse hook on its own thread (`windows.rs`); our own input is posted to the browser
//! window and never passes through it.
//!
//! Depends on: GTK/GDK/WebKitGTK, `AppKit` or Win32. Called by: vw-webview on the UI thread (install at mount, query at
//! the click).

use std::time::Duration;

/// How long after the person's last press or release checked input still waits: the platform's double-click
/// interval, which the system uses to decide whether a following press belongs to the same click.
fn recent_from(interval: Option<Duration>) -> Duration {
    interval
        .unwrap_or(Duration::from_millis(500))
        .clamp(Duration::from_millis(200), Duration::from_secs(2))
}

#[cfg(target_os = "linux")]
mod linux {
    use gdk::prelude::*;
    use gtk::prelude::*;
    use std::cell::Cell;

    thread_local! {
        static OWN_INPUT: Cell<bool> = const { Cell::new(false) };
    }

    /// Runs `send` with this thread's input marked as ours: GTK delivers it synchronously to the widget.
    pub(in crate::native_click) fn own_input(send: impl FnOnce()) {
        OWN_INPUT.with(|own| own.set(true));
        send();
        OWN_INPUT.with(|own| own.set(false));
    }

    /// Calls `observed` for every press or release (mouse, pen or touch) that reaches this webview from the
    /// window system, never for input sent by [`super::super::linux_click`].
    pub fn watch(view: &webkit2gtk::WebView, observed: impl Fn(bool) + 'static) {
        view.connect_event(move |_, event| {
            let press = matches!(
                event.event_type(),
                gdk::EventType::ButtonPress
                    | gdk::EventType::DoubleButtonPress
                    | gdk::EventType::TripleButtonPress
                    | gdk::EventType::TouchBegin
                    | gdk::EventType::KeyPress
            );
            let release = matches!(
                event.event_type(),
                gdk::EventType::ButtonRelease
                    | gdk::EventType::TouchEnd
                    | gdk::EventType::TouchCancel
                    | gdk::EventType::KeyRelease
            );
            if (press || release) && !OWN_INPUT.with(Cell::get) {
                observed(press);
            }
            glib::Propagation::Proceed
        });
    }

    /// Whether a pointer button is down right now with the pointer over this webview (window-system state).
    #[must_use]
    pub fn holding(view: &webkit2gtk::WebView) -> bool {
        let widget: &gtk::Widget = view.upcast_ref();
        let Some(window) = widget.window() else {
            return false;
        };
        let Some(device) = window
            .display()
            .default_seat()
            .and_then(|seat| seat.pointer())
        else {
            return false;
        };
        let (_, x, y, mask) = window.device_position(&device);
        mask.intersects(
            gdk::ModifierType::BUTTON1_MASK
                | gdk::ModifierType::BUTTON2_MASK
                | gdk::ModifierType::BUTTON3_MASK
                | gdk::ModifierType::BUTTON4_MASK
                | gdk::ModifierType::BUTTON5_MASK,
        ) && (0..window.width()).contains(&x)
            && (0..window.height()).contains(&y)
    }

    /// The person's double-click time from GTK settings.
    #[must_use]
    pub fn recent() -> std::time::Duration {
        let interval = gtk::Settings::default()
            .map(|settings| settings.gtk_double_click_time())
            .and_then(|ms| u64::try_from(ms).ok())
            .map(std::time::Duration::from_millis);
        super::recent_from(interval)
    }
}

#[cfg(target_os = "linux")]
pub(super) use linux::own_input;
#[cfg(target_os = "linux")]
pub use linux::{holding, recent, watch};

#[cfg(target_os = "macos")]
mod macos {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2::rc::{Retained, Weak};
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventType, NSView};
    use std::cell::{Cell, RefCell};
    use std::ptr::NonNull;

    struct Watched {
        view: Weak<NSView>,
        observed: Box<dyn Fn(bool)>,
        pressed_inside: Cell<bool>,
    }

    thread_local! {
        static WATCHED: RefCell<Vec<Watched>> = const { RefCell::new(Vec::new()) };
        static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }

    fn inside(view: &NSView, window_point: objc2_foundation::NSPoint) -> bool {
        !view.isHiddenOrHasHiddenAncestor()
            && view.mouse_inRect(
                view.convertPoint_fromView(window_point, None),
                view.bounds(),
            )
    }

    fn dispatched(event: &NSEvent, main: MainThreadMarker) {
        let kind = event.r#type();
        let keyboard = matches!(
            kind,
            NSEventType::KeyDown | NSEventType::KeyUp | NSEventType::FlagsChanged
        );
        let press = matches!(
            kind,
            NSEventType::KeyDown
                | NSEventType::FlagsChanged
                | NSEventType::LeftMouseDown
                | NSEventType::RightMouseDown
                | NSEventType::OtherMouseDown
        );
        let Some(window) = event.window(main) else {
            return;
        };
        WATCHED.with(|watched| {
            let mut watched = watched.borrow_mut();
            watched.retain(|entry| entry.view.load().is_some());
            for entry in watched.iter() {
                let Some(view) = entry.view.load() else {
                    continue;
                };
                if view.window().as_deref() != Some(&*window) {
                    continue;
                }
                let within = if keyboard {
                    window.firstResponder().is_some_and(|responder| {
                        responder.downcast_ref::<NSView>().is_some_and(|target| {
                            std::ptr::eq(target, &raw const *view) || target.isDescendantOf(&view)
                        })
                    })
                } else {
                    inside(&view, event.locationInWindow())
                };
                // A release counts where it happens, or wherever it ends a press that began in the view.
                if within || (!press && entry.pressed_inside.get()) {
                    entry.pressed_inside.set(press && within);
                    (entry.observed)(press);
                }
            }
        });
    }

    /// Calls `observed` for every mouse press or release the application dispatches inside this `WKWebView`.
    ///
    /// # Errors
    /// Off the main thread, without a view, or when `AppKit` refuses the monitor.
    pub fn watch(
        webview: *mut std::ffi::c_void,
        observed: Box<dyn Fn(bool)>,
    ) -> Result<(), String> {
        let main = MainThreadMarker::new().ok_or("pointer watch requires the main thread")?;
        // SAFETY: Tauri hands over the live WKWebView on the main thread; only a weak reference is kept.
        let view: Retained<NSView> = unsafe { Retained::retain(webview.cast::<NSView>()) }
            .ok_or("native preview webview is unavailable")?;
        MONITOR.with(|monitor| {
            if monitor.borrow().is_some() {
                return Ok(());
            }
            let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
                // SAFETY: AppKit passes a live event for the duration of this main-thread callback.
                dispatched(unsafe { event.as_ref() }, main);
                event.as_ptr()
            });
            let mask = NSEventMask::LeftMouseDown
                | NSEventMask::LeftMouseUp
                | NSEventMask::RightMouseDown
                | NSEventMask::RightMouseUp
                | NSEventMask::OtherMouseDown
                | NSEventMask::OtherMouseUp
                | NSEventMask::KeyDown
                | NSEventMask::KeyUp
                | NSEventMask::FlagsChanged;
            // SAFETY: the handler returns the event it was given (never null), as AppKit requires.
            let added =
                unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) }
                    .ok_or("AppKit refused the pointer monitor")?;
            *monitor.borrow_mut() = Some(added);
            Ok::<(), String>(())
        })?;
        WATCHED.with(|watched| {
            let mut watched = watched.borrow_mut();
            watched.retain(|entry| entry.view.load().is_some_and(|known| known != view));
            watched.push(Watched {
                view: Weak::from_retained(&view),
                observed,
                pressed_inside: Cell::new(false),
            });
        });
        Ok(())
    }

    /// Whether a mouse button is down right now with the pointer over this `WKWebView`.
    #[must_use]
    pub fn holding(webview: *mut std::ffi::c_void) -> bool {
        if NSEvent::pressedMouseButtons() == 0 {
            return false;
        }
        // SAFETY: as in `watch`, on the main thread with the live view.
        let Some(view) = (unsafe { Retained::retain(webview.cast::<NSView>()) }) else {
            return false;
        };
        view.window().is_some_and(|window| {
            inside(
                &view,
                window.convertPointFromScreen(NSEvent::mouseLocation()),
            )
        })
    }

    /// The person's double-click interval from `AppKit`.
    #[must_use]
    pub fn recent() -> std::time::Duration {
        super::recent_from(
            std::time::Duration::try_from_secs_f64(NSEvent::doubleClickInterval()).ok(),
        )
    }
}

#[cfg(target_os = "macos")]
pub use macos::{holding, recent, watch};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{holding, recent, refresh, watch};

#[cfg(test)]
mod tests {
    #[test]
    fn recent_window_follows_the_platform_interval_within_bounds() {
        use std::time::Duration;
        assert_eq!(
            super::recent_from(Some(Duration::from_millis(400))),
            Duration::from_millis(400)
        );
        assert_eq!(super::recent_from(None), Duration::from_millis(500));
        assert_eq!(
            super::recent_from(Some(Duration::ZERO)),
            Duration::from_millis(200)
        );
        assert_eq!(
            super::recent_from(Some(Duration::from_secs(9))),
            Duration::from_secs(2)
        );
    }
}
