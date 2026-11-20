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
    use std::cell::{Cell, RefCell};

    thread_local! {
        static OWN_INPUT: Cell<bool> = const { Cell::new(false) };
    }

    /// Runs `send` with this thread's input marked as ours: GTK delivers it synchronously to the widget.
    pub(in crate::native_click) fn own_input(send: impl FnOnce()) {
        OWN_INPUT.with(|own| own.set(true));
        send();
        OWN_INPUT.with(|own| own.set(false));
    }

    /// Calls `observed` for every press or release (mouse, pen or touch) or key that reaches this webview from the
    /// window system, never for input sent by [`super::super::linux_click`]. While `blocked` answers true, the
    /// person's input that [`super::block::swallow`] names is stopped before `WebKitGTK` sees it (and is not
    /// observed: the page never received it); each swallowed press is reported to `refused`.
    pub fn watch(
        view: &webkit2gtk::WebView,
        observed: impl Fn(bool) + 'static,
        blocked: super::block::Blocked,
        refused: super::block::Refused,
    ) {
        let held = RefCell::new(super::block::Held::default());
        view.connect_event(move |_, event| {
            if OWN_INPUT.with(Cell::get) {
                return glib::Propagation::Proceed;
            }
            let input = classify(event);
            let swallowed = super::block::swallow(blocked(), input, &mut held.borrow_mut());
            if super::block::refused_press(swallowed, input) {
                refused();
            }
            if swallowed {
                super::block::quit_pressed(input);
                return glib::Propagation::Stop;
            }
            match input {
                super::block::Input::Press | super::block::Input::Key { press: true, .. } => {
                    observed(true);
                }
                super::block::Input::Release | super::block::Input::Key { press: false, .. } => {
                    observed(false);
                }
                _ => {}
            }
            glib::Propagation::Proceed
        });
    }

    /// Maps a GDK event to the block rule's input kind.
    fn classify(event: &gdk::Event) -> super::block::Input {
        use super::block::Input;
        match event.event_type() {
            gdk::EventType::ButtonPress
            | gdk::EventType::DoubleButtonPress
            | gdk::EventType::TripleButtonPress
            | gdk::EventType::TouchBegin => Input::Press,
            gdk::EventType::ButtonRelease
            | gdk::EventType::TouchEnd
            | gdk::EventType::TouchCancel => Input::Release,
            gdk::EventType::MotionNotify | gdk::EventType::TouchUpdate => Input::Motion,
            gdk::EventType::Scroll
            | gdk::EventType::TouchpadSwipe
            | gdk::EventType::TouchpadPinch => Input::Wheel,
            // GTK handles the window's accelerators before the view sees a key, so no key here is a shortcut. The
            // app's Quit (Ctrl+Q) is the interface's own shortcut, not a GTK accelerator: it is recognised here.
            kind @ (gdk::EventType::KeyPress | gdk::EventType::KeyRelease) => Input::Key {
                code: u32::from(event.keycode().unwrap_or_default()),
                tab: event.keyval().is_some_and(|key| {
                    key == *gdk::keys::constants::Tab || key == *gdk::keys::constants::ISO_Left_Tab
                }),
                shortcut: false,
                quit: quit_shortcut(event) && super::block::takes_quit(),
                press: kind == gdk::EventType::KeyPress,
            },
            _ => Input::Other,
        }
    }

    /// Ctrl+Q with no other modifier (the letter by the keyboard layout, as the interface's own shortcut reads it).
    fn quit_shortcut(event: &gdk::Event) -> bool {
        let others = gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::MOD1_MASK
            | gdk::ModifierType::SUPER_MASK
            | gdk::ModifierType::META_MASK
            | gdk::ModifierType::HYPER_MASK;
        event.state().is_some_and(|state| {
            state.contains(gdk::ModifierType::CONTROL_MASK) && !state.intersects(others)
        }) && event
            .keyval()
            .is_some_and(|key| *gdk::keys::Key::from(key).to_lower() == *gdk::keys::constants::q)
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
        blocked: super::block::Blocked,
        refused: super::block::Refused,
        held: RefCell<super::block::Held>,
    }

    thread_local! {
        static WATCHED: RefCell<Vec<Watched>> = const { RefCell::new(Vec::new()) };
        static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }

    /// The virtual key code of Tab (`kVK_Tab`); Shift+Tab has the same code.
    const TAB_KEY: u16 = 48;

    fn inside(view: &NSView, window_point: objc2_foundation::NSPoint) -> bool {
        !view.isHiddenOrHasHiddenAncestor()
            && view.mouse_inRect(
                view.convertPoint_fromView(window_point, None),
                view.bounds(),
            )
    }

    /// Maps an `AppKit` event to the block rule's input kind.
    fn input_of(event: &NSEvent, kind: NSEventType) -> super::block::Input {
        use super::block::Input;
        match kind {
            NSEventType::LeftMouseDown
            | NSEventType::RightMouseDown
            | NSEventType::OtherMouseDown => Input::Press,
            NSEventType::LeftMouseUp | NSEventType::RightMouseUp | NSEventType::OtherMouseUp => {
                Input::Release
            }
            NSEventType::MouseMoved
            | NSEventType::LeftMouseDragged
            | NSEventType::RightMouseDragged
            | NSEventType::OtherMouseDragged => Input::Motion,
            NSEventType::ScrollWheel => Input::Wheel,
            NSEventType::KeyDown | NSEventType::KeyUp => Input::Key {
                code: event.keyCode().into(),
                tab: event.keyCode() == TAB_KEY,
                shortcut: event
                    .modifierFlags()
                    .contains(objc2_app_kit::NSEventModifierFlags::Command),
                // On macOS the app menu owns Command+Q; this watch does not map it.
                quit: false,
                press: kind == NSEventType::KeyDown,
            },
            _ => Input::Other,
        }
    }

    /// Observes one dispatched event; answers whether a blocked view swallows it.
    fn dispatched(event: &NSEvent, main: MainThreadMarker) -> bool {
        use super::block::Input;
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
        let release = matches!(
            kind,
            NSEventType::KeyUp
                | NSEventType::LeftMouseUp
                | NSEventType::RightMouseUp
                | NSEventType::OtherMouseUp
        );
        let input = input_of(event, kind);
        let Some(window) = event.window(main) else {
            return false;
        };
        let mut swallowed = false;
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
                // A release (and a drag) follows its press wherever it happens; other input counts in the view.
                let follows = matches!(
                    input,
                    Input::Release | Input::Motion | Input::Key { press: false, .. }
                );
                if within || follows {
                    let blocked = within && (entry.blocked)();
                    if super::block::swallow(blocked, input, &mut entry.held.borrow_mut()) {
                        swallowed = true;
                        if super::block::refused_press(true, input) {
                            (entry.refused)();
                        }
                        continue;
                    }
                }
                // A release counts where it happens, or wherever it ends a press that began in the view.
                if (press || release) && (within || (!press && entry.pressed_inside.get())) {
                    entry.pressed_inside.set(press && within);
                    (entry.observed)(press);
                }
            }
        });
        swallowed
    }

    /// Calls `observed` for every mouse press or release the application dispatches inside this `WKWebView`;
    /// drops the person's input the block rule swallows while `blocked` answers true, and reports each dropped press to
    /// `refused`.
    ///
    /// # Errors
    /// Off the main thread, without a view, or when `AppKit` refuses the monitor.
    pub fn watch(
        webview: *mut std::ffi::c_void,
        observed: Box<dyn Fn(bool)>,
        blocked: super::block::Blocked,
        refused: super::block::Refused,
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
                if dispatched(unsafe { event.as_ref() }, main) {
                    // A local monitor drops the event by answering nil.
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            });
            let mask = NSEventMask::LeftMouseDown
                | NSEventMask::LeftMouseUp
                | NSEventMask::RightMouseDown
                | NSEventMask::RightMouseUp
                | NSEventMask::OtherMouseDown
                | NSEventMask::OtherMouseUp
                | NSEventMask::LeftMouseDragged
                | NSEventMask::RightMouseDragged
                | NSEventMask::OtherMouseDragged
                | NSEventMask::MouseMoved
                | NSEventMask::ScrollWheel
                | NSEventMask::KeyDown
                | NSEventMask::KeyUp
                | NSEventMask::FlagsChanged;
            // SAFETY: the handler returns the event it was given or nil (drop), as AppKit allows.
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
                blocked,
                refused,
                held: RefCell::new(super::block::Held::default()),
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

pub mod block;

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
