//! macOS: full screen during an agent act.
//!
//! - macOS 13+: the web view's own full-screen state (`WKWebView.fullscreenState`, key-value observed) tells the act
//!   window when any document of the view, in any frame, starts entering full screen; the act window reports it
//!   and leaves it as soon as it is in (the top document's exit from the agent's world, which ends a frame's full
//!   screen too). Nothing in the page is needed to see it, and the view's preferences never change, so documents
//!   made during an act keep the full-screen API for the person. Ending it while it is still entering
//!   (`closeAllMediaPresentationsWithCompletionHandler:`) left the web view without its window on macOS 27.
//! - macOS 11-12 (no `fullscreenState`): element full screen is switched off for the act window instead (the
//!   `fullScreenEnabled` preference key Wry sets). A document made while it is off has no full-screen API until it
//!   reloads; that is the limitation of these versions.
//!
//! Depends on: objc2, objc2-foundation, objc2-web-kit, objc2-app-kit, block2. Called by: vw-webview's act window
//! on the main thread (inside Tauri's `with_webview`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSWindowStyleMask};
use objc2_foundation::{NSNumber, NSObjectProtocol, NSString, ns_string};
use objc2_web_kit::WKWebView;

/// The web view's full-screen state (`WKFullscreenState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullScreenState {
    /// Not in full screen.
    Not,
    /// Entering full screen (the transition runs).
    Entering,
    /// In full screen.
    In,
    /// Leaving full screen.
    Exiting,
}

impl FullScreenState {
    fn from_raw(raw: isize) -> Self {
        match raw {
            1 => Self::Entering,
            2 => Self::In,
            3 => Self::Exiting,
            _ => Self::Not,
        }
    }
}

/// Receives each change of a watched view's full-screen state (main thread).
pub type Changed = Box<dyn Fn(FullScreenState)>;

/// The watched view and its callback.
struct Watch {
    view: Weak<WKWebView>,
    changed: Changed,
}

define_class!(
    /// Observes one `WKWebView`'s `fullscreenState`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VibeWorkFullScreenWatch"]
    #[ivars = Watch]
    struct FullScreenWatch;

    unsafe impl NSObjectProtocol for FullScreenWatch {}

    impl FullScreenWatch {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            _key_path: Option<&NSString>,
            _object: Option<&AnyObject>,
            _change: Option<&AnyObject>,
            _context: *mut c_void,
        ) {
            if let Some(view) = self.ivars().view.load() {
                (self.ivars().changed)(state_of(&view));
            }
        }
    }
);

thread_local! {
    /// Watches by web view address (main thread only).
    static WATCHES: RefCell<HashMap<usize, Retained<FullScreenWatch>>> = RefCell::new(HashMap::new());
}

/// Whether this macOS has the web view's full-screen state (macOS 13+).
fn has_state(view: &WKWebView) -> bool {
    view.respondsToSelector(sel!(fullscreenState))
}

fn state_of(view: &WKWebView) -> FullScreenState {
    if !has_state(view) {
        return FullScreenState::Not;
    }
    // SAFETY: a plain property read on the main thread; present (checked above, macOS 13+).
    let raw: isize = unsafe { msg_send![view, fullscreenState] };
    FullScreenState::from_raw(raw)
}

fn view_of<'a>(webview: *mut c_void) -> Option<&'a WKWebView> {
    MainThreadMarker::new()?;
    // SAFETY: Tauri hands us its live `WKWebView` pointer on the main thread inside `with_webview`.
    unsafe { webview.cast::<WKWebView>().as_ref() }
}

/// Watches `webview`'s full-screen state (macOS 13+). Answers `false` when this macOS has no such state (the act
/// window then switches full screen off instead, [`set_element_full_screen`]) or off the main thread.
pub fn watch(webview: *mut c_void, changed: Changed) -> bool {
    let Some(view) = view_of(webview) else {
        return false;
    };
    if !has_state(view) {
        return false;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let watch = FullScreenWatch::alloc(mtm).set_ivars(Watch {
        view: Weak::new(view),
        changed,
    });
    // SAFETY: `init` on a freshly allocated NSObject subclass.
    let watch: Retained<FullScreenWatch> = unsafe { msg_send![super(watch), init] };
    // SAFETY: key-value observing a property WebKit documents as observable; the observer stays alive (WATCHES)
    // until `forget` removes the observation first.
    unsafe {
        let () = msg_send![view, addObserver: &*watch, forKeyPath: ns_string!("fullscreenState"), options: 0usize, context: std::ptr::null_mut::<c_void>()];
    }
    let old = WATCHES.with(|w| w.borrow_mut().insert(webview as usize, watch));
    if let Some(old) = old {
        remove(view, &old);
    }
    true
}

fn remove(view: &WKWebView, watch: &FullScreenWatch) {
    // SAFETY: removes an observation `watch` added on this view, on the main thread.
    unsafe {
        let () = msg_send![view, removeObserver: watch, forKeyPath: ns_string!("fullscreenState")];
    }
}

/// Ends the watch of a closing `webview`.
pub fn forget(webview: *mut c_void) {
    let watch = WATCHES.with(|w| w.borrow_mut().remove(&(webview as usize)));
    if let (Some(watch), Some(view)) = (watch, view_of(webview)) {
        remove(view, &watch);
    }
}

/// How many of the application's windows are in macOS full screen (for the act window's measurement log).
#[must_use]
pub fn windows_in_full_screen() -> usize {
    let Some(mtm) = MainThreadMarker::new() else {
        return 0;
    };
    NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .filter(|w| w.styleMask().contains(NSWindowStyleMask::FullScreen))
        .count()
}

/// macOS 11-12: sets element full screen of `webview` (Tauri's `PlatformWebview::inner()`, a `WKWebView`). Applies to
/// the current documents at once. Does nothing off the main thread or without a webview.
pub fn set_element_full_screen(webview: *mut c_void, enabled: bool) {
    let Some(view) = view_of(webview) else {
        return;
    };
    // SAFETY: main-thread writes of the webview's own preferences; `fullScreenEnabled` is the key Wry sets the same way.
    unsafe {
        let prefs = view.configuration().preferences();
        let value = NSNumber::numberWithBool(enabled);
        let () = msg_send![&*prefs, setValue: &*value, forKey: ns_string!("fullScreenEnabled")];
    }
}

/// Whether the act window must switch full screen off on this view (no full-screen state to watch: macOS 11-12).
#[must_use]
pub fn needs_switch(webview: *mut c_void) -> bool {
    view_of(webview).is_some_and(|view| !has_state(view))
}
