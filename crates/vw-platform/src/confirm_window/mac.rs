//! macOS confirmation window (see `confirm_window.rs`): our own `NSAlert` as a sheet or as a window of its own (each kind
//! one at a time), answered through its buttons' own action, with a close from outside.
//!
//! Every key is set here (Return on the declining button only, Escape through a key monitor), and the window can be
//! closed from outside. Nothing here keeps or restores another window's focus: `AppKit` gives the key back to the
//! parent when a sheet ends, so with several confirmations in a row only the one shown can take the key.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertStyle, NSButton, NSEvent, NSEventMask, NSResponder, NSView,
    NSWindow,
};
use objc2_foundation::NSString;

use super::{ConfirmSpec, Reply};

/// The key code of Escape on every Mac keyboard (`kVK_Escape`).
const ESCAPE: u16 = 53;

/// One shown or waiting confirmation.
struct Open {
    alert: Retained<NSAlert>,
    /// The buttons' action target; lives as long as the alert.
    _target: Retained<Target>,
    /// The local key monitor that turns Escape in this alert into "declined"; removed when it ends.
    monitor: Option<Retained<AnyObject>>,
    /// The sheet's parent; `None`: the alert is a window of its own.
    parent: Option<Retained<NSWindow>>,
    reply: Option<Reply>,
}

thread_local! {
    /// Every confirmation of this process, by id (main thread only, like everything `AppKit`).
    static OPEN: RefCell<HashMap<u64, Open>> = RefCell::new(HashMap::new());
    /// The confirmation shown now, and the ones waiting for it to end, oldest first.
    static SHOWN: RefCell<Option<u64>> = const { RefCell::new(None) };
    static WAITING: RefCell<VecDeque<u64>> = const { RefCell::new(VecDeque::new()) };
    /// Confirmations as windows of their own, beside the sheets: also one at a time, so a new one never appears
    /// where the person is about to click another.
    static PANEL: RefCell<Option<u64>> = const { RefCell::new(None) };
    static PANELS_WAITING: RefCell<VecDeque<u64>> = const { RefCell::new(VecDeque::new()) };
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The target of every button of one alert.
struct TargetIvars {
    id: u64,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; `Target` adds no Drop and is used on the main thread only.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VibeWorkConfirmationTarget"]
    #[ivars = TargetIvars]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        /// Every button's action (a click, Return, Space, an accessibility press): the tag is the button's index.
        #[unsafe(method(pressed:))]
        fn pressed(&self, sender: &NSButton) {
            finish(self.ivars().id, usize::try_from(sender.tag()).ok());
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, id: u64) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { id });
        // SAFETY: NSObject's designated initializer on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

/// Ends confirmation `id` with `answer` and shows the next waiting one. An id that already ended does nothing, so
/// a press after a close (or a second press) is never reported.
fn finish(id: u64, answer: Option<usize>) {
    let Some(mut open) = OPEN.with(|all| all.borrow_mut().remove(&id)) else {
        return;
    };
    if let Some(monitor) = open.monitor.take() {
        // SAFETY: the object addLocalMonitorForEventsMatchingMask returned for this alert, removed once.
        unsafe { NSEvent::removeMonitor(&monitor) };
    }
    let window = open.alert.window();
    if PANEL.with(|panel| *panel.borrow() == Some(id)) {
        PANEL.with(|panel| *panel.borrow_mut() = None);
        window.orderOut(None);
        window.close();
    } else if SHOWN.with(|shown| *shown.borrow() == Some(id)) {
        SHOWN.with(|shown| *shown.borrow_mut() = None);
        match &open.parent {
            Some(parent) => parent.endSheet(&window),
            None => window.orderOut(None),
        }
        window.close();
    } else {
        WAITING.with(|waiting| waiting.borrow_mut().retain(|other| *other != id));
        PANELS_WAITING.with(|waiting| waiting.borrow_mut().retain(|other| *other != id));
    }
    if let Some(reply) = open.reply.take() {
        reply(answer);
    }
    show_next();
    // The next window of its own may belong to another chat than the one in front: it opens behind the app's
    // windows, like a window that was already open there, and comes forward when its chat is shown (`raise`).
    show_next_panel(false);
}

/// Shows the oldest waiting confirmation when none is shown.
fn show_next() {
    if SHOWN.with(|shown| shown.borrow().is_some()) {
        return;
    }
    let Some(id) = WAITING.with(|waiting| waiting.borrow_mut().pop_front()) else {
        return;
    };
    // Taken out of the table first: `AppKit` is never called while the table is borrowed.
    let Some((window, parent, declining)) = OPEN.with(|all| {
        all.borrow().get(&id).map(|open| {
            (
                open.alert.window(),
                open.parent.clone(),
                open.alert.buttons().firstObject(),
            )
        })
    }) else {
        show_next();
        return;
    };
    SHOWN.with(|shown| *shown.borrow_mut() = Some(id));
    // Only sheets wait here; a window of its own is shown by `show_next_panel`.
    if let Some(parent) = parent {
        parent.beginSheet_completionHandler(&window, None);
    }
    // With Full Keyboard Access `AppKit` starts the focus on the leftmost button (an approving one), where a stray
    // Space would approve: the focus starts on the declining button instead, as on Windows and Linux.
    if let Some(declining) = declining {
        let responder: &NSResponder = &declining;
        window.makeFirstResponder(Some(responder));
    }
}

/// The alert for `spec`, its buttons tagged with their index and aimed at a new [`Target`].
fn build(
    mtm: MainThreadMarker,
    id: u64,
    spec: &ConfirmSpec,
) -> (Retained<NSAlert>, Retained<Target>) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.setMessageText(&NSString::from_str(&spec.heading));
    alert.setInformativeText(&NSString::from_str(&spec.body));
    let target = Target::new(mtm, id);
    for (index, label) in spec.buttons.iter().enumerate() {
        let button = alert.addButtonWithTitle(&NSString::from_str(label));
        button.setTag(isize::try_from(index).unwrap_or(isize::MAX));
        // Only the first (declining) button has a key: Return. `AppKit` would have given Escape to a "Cancel" button
        // (taking Return away from it) and nothing to "Don't allow" or "Keep"; Escape is the key monitor's instead.
        button.setKeyEquivalent(&NSString::from_str(if index == 0 { "\r" } else { "" }));
        // SAFETY: `target` implements `pressed:` and is kept in the same `Open` entry as the alert, so it outlives
        // every use the button makes of it (buttons do not retain their target).
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(pressed:)));
        }
    }
    alert.layout();
    let window = alert.window();
    if let Some(declining) = alert.buttons().firstObject() {
        let view: &NSView = &declining;
        window.setInitialFirstResponder(Some(view));
    }
    // The question is the window's title for accessibility (a sheet shows no title bar, and AppKit reports a
    // sheet's own title as empty): VoiceOver names the alert by it before reading the texts.
    let question = NSString::from_str(&spec.heading);
    window.setTitle(&question);
    window.setAccessibilityTitle(Some(&question));
    // SAFETY: the alert owns its window and `finish` closes it explicitly; it must not also free itself on close.
    unsafe { window.setReleasedWhenClosed(false) };
    (alert, target)
}

/// A local key monitor: Escape in alert `id`'s window, while it is the shown one, declines it (the event is used
/// up); every other key event goes on unchanged.
fn escape_monitor(id: u64) -> Option<Retained<AnyObject>> {
    let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: `AppKit` hands a local monitor a valid event, on the main thread.
        let key = unsafe { event.as_ref() };
        let shown = SHOWN.with(|shown| *shown.borrow() == Some(id))
            || PANEL.with(|panel| *panel.borrow() == Some(id));
        if key.keyCode() == ESCAPE && shown {
            let ours = OPEN.with(|all| {
                all.borrow()
                    .get(&id)
                    .is_some_and(|open| open.alert.window().windowNumber() == key.windowNumber())
            });
            if ours {
                finish(id, None);
                return std::ptr::null_mut();
            }
        }
        event.as_ptr()
    });
    // SAFETY: `AppKit` copies the block and keeps it until the monitor is removed (in `finish`).
    unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block) }
}

/// Shows `spec` as a sheet on `parent` (an `NSWindow` of this process), or with a null `parent` as a window of its
/// own beside the sheets (each kind one at a time, in the order asked), and answers
/// `reply` once. Must be called on the main thread; returns the id for [`close`].
///
/// # Errors
/// Not on the main thread, or not 1 to 3 buttons (nothing is shown; `reply` is not called).
pub fn show(parent: *mut c_void, spec: &ConfirmSpec, reply: Reply) -> Result<u64, String> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("the confirmation window must be shown on the main thread".to_owned());
    };
    if !(1..=3).contains(&spec.buttons.len()) {
        return Err("a confirmation window has 1 to 3 buttons".to_owned());
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (alert, target) = build(mtm, id, spec);
    // SAFETY: the caller hands a live `NSWindow` of this process (Tauri's `ns_window()`, read on this main thread) or
    // null; retaining it keeps it alive for the sheet.
    let parent = (!parent.is_null())
        .then(|| unsafe { Retained::retain(parent.cast::<NSWindow>()) })
        .flatten();
    OPEN.with(|all| {
        all.borrow_mut().insert(
            id,
            Open {
                alert,
                _target: target,
                monitor: None,
                parent,
                reply: Some(reply),
            },
        );
    });
    let monitor = escape_monitor(id);
    OPEN.with(|all| {
        if let Some(open) = all.borrow_mut().get_mut(&id) {
            open.monitor = monitor;
        }
    });
    if OPEN.with(|all| {
        all.borrow()
            .get(&id)
            .is_some_and(|open| open.parent.is_none())
    }) {
        PANELS_WAITING.with(|waiting| waiting.borrow_mut().push_back(id));
        show_next_panel(true);
    } else {
        WAITING.with(|waiting| waiting.borrow_mut().push_back(id));
        show_next();
    }
    Ok(id)
}

/// Shows the oldest waiting window of its own when none is shown, in `front` (else behind the app's windows, not
/// key); the app's windows stay usable.
fn show_next_panel(front: bool) {
    if PANEL.with(|panel| panel.borrow().is_some()) {
        return;
    }
    let Some(id) = PANELS_WAITING.with(|waiting| waiting.borrow_mut().pop_front()) else {
        return;
    };
    let Some((window, declining)) = OPEN.with(|all| {
        all.borrow()
            .get(&id)
            .map(|open| (open.alert.window(), open.alert.buttons().firstObject()))
    }) else {
        show_next_panel(front);
        return;
    };
    PANEL.with(|panel| *panel.borrow_mut() = Some(id));
    // An ordinary window level, not a floating one: a click on the main window brings it (and a sheet the person
    // opens there) in front of this prompt, so this prompt's buttons never lie over the person's own sheet.
    window.center();
    if front {
        window.makeKeyAndOrderFront(None);
    } else {
        window.orderBack(None);
    }
    if let Some(declining) = declining {
        let responder: &NSResponder = &declining;
        window.makeFirstResponder(Some(responder));
    }
}

/// Brings confirmation `id` to the front when it is a window of its own (main thread only; never answers). Those are
/// shown one at a time: when `id` still waits behind another one, the shown one steps back into the queue (first in
/// line again) and `id` is shown in its place, so the prompt that comes forward is always the one asked for, never
/// another chat's. False for a sheet (it is on the main window already), an id that ended, or off the main thread.
#[must_use]
pub fn raise(id: u64) -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    let own = OPEN.with(|all| {
        all.borrow()
            .get(&id)
            .is_some_and(|open| open.parent.is_none())
    });
    if !own {
        return false;
    }
    let shown = PANEL.with(|panel| *panel.borrow());
    if shown != Some(id) {
        let queued = PANELS_WAITING.with(|waiting| {
            let mut waiting = waiting.borrow_mut();
            let before = waiting.len();
            waiting.retain(|other| *other != id);
            waiting.len() != before
        });
        if !queued {
            return false;
        }
        if let Some(current) = shown {
            if let Some(window) =
                OPEN.with(|all| all.borrow().get(&current).map(|open| open.alert.window()))
            {
                window.orderOut(None);
            }
            PANELS_WAITING.with(|waiting| waiting.borrow_mut().push_front(current));
            PANEL.with(|panel| *panel.borrow_mut() = None);
        }
        PANELS_WAITING.with(|waiting| waiting.borrow_mut().push_front(id));
        show_next_panel(true);
        return true;
    }
    let Some(window) = OPEN.with(|all| all.borrow().get(&id).map(|open| open.alert.window()))
    else {
        return false;
    };
    window.makeKeyAndOrderFront(None);
    true
}

/// Closes confirmation `id` (its sheet goes away, or it leaves the queue) and answers its reply with `None`. Main
/// thread only (elsewhere nothing happens); an id that already ended does nothing.
pub fn close(id: u64) {
    if MainThreadMarker::new().is_some() {
        finish(id, None);
    }
}
