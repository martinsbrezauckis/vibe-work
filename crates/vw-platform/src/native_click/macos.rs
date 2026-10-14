//! `AppKit` mouse delivery into the current `WKWebView`'s hit-tested native view.
//! Depends on: `WKWebView` and `AppKit`. Called by: `native_click` on Tauri's main thread.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::ClickReply;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView};
use objc2_foundation::{
    NSError, NSJSONSerialization, NSJSONWritingOptions, NSPoint, NSProcessInfo, NSString,
    NSUTF8StringEncoding,
};
use objc2_web_kit::WKWebView;

/// Sends a left-button press and release inside a visible, key-window `WKWebView`.
///
/// # Errors
/// Refuses an absent view, hidden or inactive window, and a point outside the view's hit tree.
pub fn click(
    webview: *mut std::ffi::c_void,
    point: (f64, f64),
    sent: &AtomicBool,
) -> Result<(), String> {
    let _main = MainThreadMarker::new().ok_or("native click requires the main thread")?;
    // SAFETY: Tauri gives this live WKWebView pointer inside with_webview on the main thread. Retaining it keeps the
    // view alive through both AppKit messages; no global event tap or other app receives the events.
    let view = unsafe { Retained::retain(webview.cast::<WKWebView>()) }
        .ok_or("native preview webview is unavailable")?;
    click_view(&view, point, sent)
}

fn click_view(view: &WKWebView, point: (f64, f64), sent: &AtomicBool) -> Result<(), String> {
    let main = MainThreadMarker::new().ok_or("native click requires the main thread")?;
    if !NSApplication::sharedApplication(main).isActive() {
        return Err("native preview application is not active".to_owned());
    }
    let base: &NSView = view;
    if base.isHiddenOrHasHiddenAncestor() {
        return Err("native preview is hidden".to_owned());
    }
    let window = base.window().ok_or("native preview has no window")?;
    if !window.isVisible() || !window.isKeyWindow() {
        return Err("native preview window is not key".to_owned());
    }
    let bounds = base.bounds();
    let (x, y) = super::map_point(point, (bounds.size.width, bounds.size.height))?;
    let local = NSPoint::new(
        bounds.origin.x + x,
        bounds.origin.y
            + if base.isFlipped() {
                y
            } else {
                bounds.size.height - y
            },
    );
    let window_point = base.convertPoint_toView(local, None);
    let content = window
        .contentView()
        .ok_or("native preview window has no content view")?;
    // Apple defines hitTest's argument in the receiver's *superview* coordinates. A content view may itself be
    // inside an AppKit frame view, so map from window space when that parent exists.
    // SAFETY: the retained content view and window stay alive through this synchronous hit test.
    let parent = unsafe { content.superview() };
    let hit_point = parent.as_ref().map_or(window_point, |parent| {
        parent.convertPoint_fromView(window_point, None)
    });
    let hit = content
        .hitTest(hit_point)
        .ok_or("native preview point hit no view")?;
    if !std::ptr::eq(&raw const *hit, base) && !hit.isDescendantOf(base) {
        return Err("native preview point is occluded".to_owned());
    }
    let number = window.windowNumber();
    let time = NSProcessInfo::processInfo().systemUptime();
    let moved = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::MouseMoved,
        window_point,
        NSEventModifierFlags::empty(),
        time,
        number,
        None,
        0,
        0,
        0.0,
    )
    .ok_or("AppKit could not create mouse move")?;
    let down = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::LeftMouseDown,
        window_point,
        NSEventModifierFlags::empty(),
        time,
        number,
        None,
        0,
        1,
        1.0,
    )
    .ok_or("AppKit could not create mouse press")?;
    let up = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::LeftMouseUp,
        window_point,
        NSEventModifierFlags::empty(),
        time,
        number,
        None,
        0,
        1,
        0.0,
    )
    .ok_or("AppKit could not create mouse release")?;
    hit.mouseMoved(&moved);
    sent.store(true, Ordering::Release);
    hit.mouseDown(&down);
    hit.mouseUp(&up);
    Ok(())
}

/// Evaluates the final page guard, then sends native input in `WebKit`'s main-thread completion.
pub fn click_checked(
    webview: *mut std::ffi::c_void,
    point: (f64, f64),
    script: &str,
    sent: Arc<AtomicBool>,
    allowed: super::ClickPermission,
    reply: ClickReply,
) {
    let Some(_main) = MainThreadMarker::new() else {
        let _ = reply.send(Err(
            "Not run: native click requires the main thread".to_owned()
        ));
        return;
    };
    // SAFETY: Tauri hands us the live WKWebView on its main thread. Retaining it keeps the view alive until the
    // WebKit completion; Apple documents that completion runs on the app's main thread.
    let Some(view) = (unsafe { Retained::retain(webview.cast::<WKWebView>()) }) else {
        let _ = reply.send(Err("Not run: native preview is unavailable".to_owned()));
        return;
    };
    let reply = RefCell::new(Some(reply));
    let callback_view = view.clone();
    let callback = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
        let Some(reply) = reply.borrow_mut().take() else {
            return;
        };
        if reply.is_closed() {
            return;
        }
        let result = if !error.is_null() || value.is_null() {
            Err("Not run: final page validation failed".to_owned())
        } else {
            // SAFETY: WebKit supplied this object for the lifetime of the callback. NSJSONSerialization's fragment
            // form distinguishes JavaScript boolean true from number 1 or a truthy string.
            let json = unsafe {
                NSJSONSerialization::dataWithJSONObject_options_error(
                    &*value,
                    NSJSONWritingOptions::FragmentsAllowed,
                )
            }
            .ok()
            .and_then(|data| {
                NSString::initWithData_encoding(NSString::alloc(), &data, NSUTF8StringEncoding)
            })
            .map(|text| text.to_string());
            if json.as_deref().is_some_and(super::validation_is_true) {
                if reply.is_closed() {
                    Err("Not run: native click was canceled".to_owned())
                } else if allowed.as_ref().is_some_and(|allowed| !allowed()) {
                    Err("Not run: browser permission changed before native input".to_owned())
                } else {
                    click_view(&callback_view, point, &sent)
                }
            } else {
                Err("Not run: the page target changed before native input".to_owned())
            }
        };
        let _ = reply.send(result);
    });
    // SAFETY: the retained view, callback block and string are valid for this main-thread WebKit call.
    unsafe {
        view.evaluateJavaScript_completionHandler(&NSString::from_str(script), Some(&callback));
    }
}
