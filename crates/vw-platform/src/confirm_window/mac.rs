//! macOS confirmation window (see `confirm_window.rs`): our own `NSAlert` as a sheet or as a window of its own (each kind
//! one at a time), answered through its buttons' own action, with a close from outside.
//!
//! Every key is set here (Return on the declining button only, Escape through a key monitor), and the window can be
//! closed from outside. Nothing here keeps or restores another window's focus: `AppKit` gives the key back to the
//! parent when a sheet ends, so with several confirmations in a row only the one shown can take the key.
//!
//! A window of its own (an agent's question) never comes in front of another app: asked while Vibe Work is not the
//! active app it is not shown yet (the Dock badge says it waits) and comes forward, key, when the person makes Vibe
//! Work active (also over the main window a Dock click shows: `raise_shown`) or shows its chat (`raise`); shown in
//! front it is a child window of the app's main window, so it stays above that window. When the person answers one, the
//! next waiting one comes forward, key, in its place. A key press meant for it while
//! it cannot be seen (behind another window) never answers it: the press is dropped and the window comes forward
//! instead.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertStyle, NSApplication,
    NSApplicationDidBecomeActiveNotification, NSButton, NSEvent, NSEventMask, NSResponder, NSView,
    NSWindow, NSWindowOrderingMode,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSString, NSTimer};

use super::{ConfirmSpec, Reply};

/// The key code of Escape on every Mac keyboard (`kVK_Escape`).
const ESCAPE: u16 = 53;
/// `NSWindowOcclusionStateVisible`: some part of the window can be seen on screen.
const OCCLUSION_VISIBLE: usize = 1 << 1;
/// After a window of its own comes forward by itself (the app became active, a hidden key press), presses on it are
/// ignored this long, so a click or key meant for something else at that moment never answers it.
const SETTLE: Duration = Duration::from_millis(500);
/// When after the app became active (or a raise) the shown window of its own is checked again: the app's own reopen
/// handling (a Dock click shows the main window) runs after the activation and can cover it.
const RECHECK_SECONDS: [f64; 2] = [0.3, 1.0];

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
    /// The observer that brings the shown window of its own forward when the app becomes active (added once).
    static ACTIVATION: RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>> =
        const { RefCell::new(None) };
    /// The click monitor that lets a click on the main window bring it in front of the prompt (added once).
    static CLICKS: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    /// When a window of its own last came forward by itself ([`SETTLE`]).
    static BROUGHT_FORWARD: Cell<Option<Instant>> = const { Cell::new(None) };
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
            let settling = BROUGHT_FORWARD.with(Cell::get).is_some_and(|at| at.elapsed() < SETTLE);
            if settling && PANEL.with(|panel| *panel.borrow() == Some(self.ivars().id)) {
                return;
            }
            finish(self.ivars().id, usize::try_from(sender.tag()).ok(), Ended::ByPerson);
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

/// How a confirmation ended: the person answered it (a button, Return, Escape), or it was closed from outside.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ended {
    ByPerson,
    FromOutside,
}

/// Ends confirmation `id` with `answer` and shows the next waiting one. An id that already ended does nothing, so
/// a press after a close (or a second press) is never reported.
fn finish(id: u64, answer: Option<usize>, ended: Ended) {
    let Some(mut open) = OPEN.with(|all| all.borrow_mut().remove(&id)) else {
        return;
    };
    if let Some(monitor) = open.monitor.take() {
        // SAFETY: the object addLocalMonitorForEventsMatchingMask returned for this alert, removed once.
        unsafe { NSEvent::removeMonitor(&monitor) };
    }
    let window = open.alert.window();
    let was_panel = PANEL.with(|panel| *panel.borrow() == Some(id));
    if was_panel {
        PANEL.with(|panel| *panel.borrow_mut() = None);
        detach(&window);
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
    // The person just answered a window of its own: the next one takes its place in front, key, so the next Return
    // answers it (after the settle time, so a repeated press never does). Ordered behind, macOS 27 shows it over the
    // main window without the key, where a Return goes to the main window. Closed from outside,
    // the next one may belong to another chat than the one in front: it opens behind the app's windows and comes
    // forward when its chat is shown (`raise`).
    show_next_panel(was_panel && ended == Ended::ByPerson);
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
        // A prompt attached to the main window would stay above this sheet: it goes back to the ordinary order.
        if let Some(panel) = shown_panel() {
            detach(&panel);
        }
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
/// up); a key press for it while it is a window of its own that cannot be seen is used up and brings it forward
/// instead (never an answer to a question the person cannot see); every other key event goes on unchanged.
fn escape_monitor(id: u64) -> Option<Retained<AnyObject>> {
    let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: `AppKit` hands a local monitor a valid event, on the main thread.
        let key = unsafe { event.as_ref() };
        let panel = PANEL.with(|panel| *panel.borrow() == Some(id));
        if panel {
            let hidden = OPEN.with(|all| {
                all.borrow().get(&id).and_then(|open| {
                    let window = open.alert.window();
                    (window.windowNumber() == key.windowNumber() && !seen(&window))
                        .then_some(window)
                })
            });
            if let Some(window) = hidden {
                bring_forward(&window);
                return std::ptr::null_mut();
            }
        }
        let shown = SHOWN.with(|shown| *shown.borrow() == Some(id)) || panel;
        if key.keyCode() == ESCAPE && shown {
            let ours = OPEN.with(|all| {
                all.borrow()
                    .get(&id)
                    .is_some_and(|open| open.alert.window().windowNumber() == key.windowNumber())
            });
            if ours {
                finish(id, None, Ended::ByPerson);
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

/// Brings a window of its own forward, key, by itself (not by the person's click on it): presses settle first.
/// While the app shows a main window, the prompt becomes that window's child window: `AppKit` then keeps it directly
/// above the main window whatever orders the main window forward afterwards (the app's Dock-click handling shows and
/// focuses the main window after the activation), and a child window never comes over another app's windows. The
/// person's click on the main window ends that ([`watch_clicks`]), so the main window and a sheet opened there come in
/// front of the prompt as with any other window.
fn bring_forward(window: &NSWindow) {
    BROUGHT_FORWARD.with(|at| at.set(Some(Instant::now())));
    let Some(main) = shown_main_window(window) else {
        detach(window);
        window.makeKeyAndOrderFront(None);
        window.orderFrontRegardless();
        return;
    };
    let attached = window
        .parentWindow()
        .is_some_and(|parent| parent.windowNumber() == main.windowNumber());
    if !attached {
        detach(window);
        // SAFETY: both are live windows of this process on the main thread; the child is removed again in
        // `detach` before it is closed or ordered out.
        unsafe { main.addChildWindow_ordered(window, NSWindowOrderingMode::Above) };
    }
    window.makeKeyAndOrderFront(None);
}

/// The app's visible main (document) window other than `prompt`: the one `AppKit` calls main, else a visible window
/// that can become main (the app has one such window).
fn shown_main_window(prompt: &NSWindow) -> Option<Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(MainThreadMarker::from(prompt));
    let usable = |candidate: &NSWindow| {
        candidate.windowNumber() != prompt.windowNumber()
            && candidate.isVisible()
            && !candidate.isMiniaturized()
            && candidate.canBecomeMainWindow()
    };
    if let Some(main) = app.mainWindow().filter(|main| usable(main)) {
        return Some(main);
    }
    app.windows().iter().find(|candidate| usable(candidate))
}

/// Takes a prompt off the window it was attached to by [`bring_forward`] (before it is ordered out or closed).
fn detach(window: &NSWindow) {
    if let Some(parent) = window.parentWindow() {
        parent.removeChildWindow(window);
    }
}

/// A press of a mouse button in the window the shown prompt is attached to takes the prompt off it first, so the
/// click brings that window (and a sheet the person then opens there) in front of the prompt as usual. Every event
/// goes on unchanged. Added once, on the main thread.
fn watch_clicks() {
    if CLICKS.with(|monitor| monitor.borrow().is_some()) {
        return;
    }
    let block = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: `AppKit` hands a local monitor a valid event, on the main thread.
        let click = unsafe { event.as_ref() };
        if let Some(window) = shown_panel()
            && window
                .parentWindow()
                .is_some_and(|parent| parent.windowNumber() == click.windowNumber())
        {
            detach(&window);
        }
        event.as_ptr()
    });
    let mask =
        NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown;
    // SAFETY: `AppKit` copies the block and keeps it as long as the monitor exists (never removed: it lives as long as
    // the app); the block touches only main-thread state.
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
    CLICKS.with(|slot| *slot.borrow_mut() = monitor);
}

/// The shown window of its own, when there is one.
fn shown_panel() -> Option<Retained<NSWindow>> {
    PANEL
        .with(|panel| *panel.borrow())
        .and_then(|id| OPEN.with(|all| all.borrow().get(&id).map(|open| open.alert.window())))
}

/// Checks again shortly ([`RECHECK_SECONDS`]) that the shown window of its own can be seen while the app is active,
/// and brings it forward when another window of the app covered it meanwhile.
fn check_soon() {
    for delay in RECHECK_SECONDS {
        check_after(delay);
    }
}

fn check_after(delay: f64) {
    let block = RcBlock::new(|_: NonNull<NSTimer>| {
        let Some(window) = shown_panel() else {
            return;
        };
        let active = NSApplication::sharedApplication(MainThreadMarker::from(&*window)).isActive();
        if active && !seen(&window) {
            bring_forward(&window);
        }
    });
    // SAFETY: called on the main thread (its run loop fires the timer there); the timer copies the block and the
    // block touches only main-thread state.
    let _ = unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(delay, false, &block) };
}

/// True when part of `window` can be seen on screen now (ordered in, not wholly behind other windows). A prompt
/// attached to the shown main window ([`bring_forward`]) is above it by `AppKit`'s own ordering; its occlusion state
/// was measured to stay "not visible" there (macOS 27), which would drop every key meant for it.
fn seen(window: &NSWindow) -> bool {
    if !window.isVisible() {
        return false;
    }
    let above_main = window
        .parentWindow()
        .is_some_and(|parent| parent.isVisible() && !parent.isMiniaturized());
    above_main || window.occlusionState().0 & OCCLUSION_VISIBLE != 0
}

/// Brings the shown window of its own forward, key, whenever the app becomes active (a Dock click, Command-Tab, a
/// click in one of its windows): it may have been asked while another app was active, or gone behind the main window.
/// Added once, on the main thread, the first time a window of its own is shown.
fn watch_activation(_on_main_thread: MainThreadMarker) {
    if ACTIVATION.with(|observer| observer.borrow().is_some()) {
        return;
    }
    let block = RcBlock::new(|_: NonNull<NSNotification>| {
        let Some(window) = PANEL
            .with(|panel| *panel.borrow())
            .and_then(|id| OPEN.with(|all| all.borrow().get(&id).map(|open| open.alert.window())))
        else {
            return;
        };
        bring_forward(&window);
        check_soon();
    });
    // SAFETY: the notification is posted on the main thread (no queue: the block runs where it is posted), the block
    // touches only main-thread state, and the center keeps the copied block until the observer is removed (never: it
    // lives as long as the app).
    let observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSApplicationDidBecomeActiveNotification),
            None,
            None,
            &block,
        )
    };
    ACTIVATION.with(|slot| *slot.borrow_mut() = Some(observer));
}

/// Shows the oldest waiting window of its own when none is shown, in `front` (else behind the app's windows, not
/// key); the app's windows stay usable. Never in front of another app: while Vibe Work is not active it waits behind
/// (and [`watch_activation`] brings it forward when the person makes the app active).
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
    let mtm = MainThreadMarker::from(&*window);
    watch_activation(mtm);
    watch_clicks();
    let active = NSApplication::sharedApplication(mtm).isActive();
    // An ordinary window level, not a floating one: a click on the main window brings it (and a sheet the person
    // opens there) in front of this prompt, so this prompt's buttons never lie over the person's own sheet.
    window.center();
    // While another app is active the window is not ordered in at all: even `orderBack` placed a new window of an
    // inactive app above the active app's windows (macOS 27). The activation observer shows it.
    if front && active {
        // It appears by itself, maybe under a click meant for something else: presses settle first.
        bring_forward(&window);
    } else if active {
        window.orderBack(None);
    }
    if let Some(declining) = declining {
        let responder: &NSResponder = &declining;
        window.makeFirstResponder(Some(responder));
    }
}

/// Brings confirmation `id` to the front when it is a window of its own (main thread only; never answers; never in
/// front of another app: while Vibe Work is not active it comes forward when the app becomes active). Those are
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
                detach(&window);
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
    // While another app is active the window waits (its chat was shown in the background, a new ask of the shown
    // chat): it comes forward when the person makes Vibe Work active (`watch_activation`).
    if NSApplication::sharedApplication(MainThreadMarker::from(&*window)).isActive() {
        bring_forward(&window);
        check_soon();
    }
    true
}

/// Brings the shown window of its own forward (key) when Vibe Work is the active app; the app calls it after it showed
/// its main window for the person (a Dock click), which would otherwise cover the waiting question. Main thread only;
/// never answers. False when nothing is shown, the app is not active, or off the main thread.
#[must_use]
pub fn raise_shown() -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    let Some(window) = shown_panel() else {
        return false;
    };
    if !NSApplication::sharedApplication(MainThreadMarker::from(&*window)).isActive() {
        return false;
    }
    bring_forward(&window);
    check_soon();
    true
}

/// Closes confirmation `id` (its sheet goes away, or it leaves the queue) and answers its reply with `None`. Main
/// thread only (elsewhere nothing happens); an id that already ended does nothing.
pub fn close(id: u64) {
    if MainThreadMarker::new().is_some() {
        finish(id, None, Ended::FromOutside);
    }
}
