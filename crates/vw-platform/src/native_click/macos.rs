//! `AppKit` mouse delivery into the current `WKWebView`'s hit-tested native view, and a short main-thread delay.
//! Depends on: `WKWebView` and `AppKit`. Called by: `native_click` on Tauri's main thread.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSScreen, NSTrackingAreaOptions,
    NSView,
};
use objc2_core_graphics::{CGEvent, CGEventField};
use objc2_foundation::{
    NSPoint, NSPointInRect, NSProcessInfo, NSRunLoop, NSRunLoopCommonModes, NSTimer,
};
use objc2_web_kit::WKWebView;

#[path = "macos_wheel.rs"]
mod wheel;

/// Sends a left-button press and release inside a visible, key-window `WKWebView`.
///
/// # Errors
/// Refuses an absent view, hidden or inactive window, and a point outside the view's hit tree.
pub fn click(
    webview: *mut std::ffi::c_void,
    point: (f64, f64),
    sent: &AtomicBool,
) -> Result<(), String> {
    click_options(webview, point, sent, (0, 1, 0, None))
}

/// Sends a native button/count/modifier sequence to this view only.
///
/// # Errors
/// Invalid options, an absent view, hidden/inactive window or an occluded point.
pub fn click_options(
    webview: *mut std::ffi::c_void,
    point: (f64, f64),
    sent: &AtomicBool,
    options: (u8, u8, u8, Option<(i32, i32)>),
) -> Result<(), String> {
    let wheel = super::macos_wheel_plan::deltas(options)?;
    let options = (options.0, options.1, options.2);
    if options != (0, 0, 0) && (options.0 > 2 || !(1..=3).contains(&options.1) || options.2 > 15) {
        return Err("invalid native click options".into());
    }
    let _main = MainThreadMarker::new().ok_or("native click requires the main thread")?;
    // SAFETY: Tauri gives this live WKWebView pointer inside with_webview on the main thread. Retaining it keeps the
    // view alive through both AppKit messages; no global event tap or other app receives the events.
    let view = unsafe { Retained::retain(webview.cast::<WKWebView>()) }
        .ok_or("native preview webview is unavailable")?;
    click_view(&view, point, sent, options, wheel)
}

/// `Err(`[`super::MENU_OPEN`]`)` while the main run loop runs `AppKit`'s event-tracking mode, the mode of an open menu (also of a
/// window drag or live resize). Call on the main thread, right before native input.
///
/// # Errors
/// [`super::MENU_OPEN`].
pub fn refuse_open_menu() -> Result<(), String> {
    let mode = NSRunLoop::currentRunLoop().currentMode();
    // SAFETY: an immutable AppKit constant string, read on the main thread.
    let tracking = unsafe { objc2_app_kit::NSEventTrackingRunLoopMode };
    if mode.is_some_and(|mode| mode.isEqualToString(tracking)) {
        Err(super::MENU_OPEN.to_owned())
    } else {
        Ok(())
    }
}

fn click_view(
    view: &WKWebView,
    point: (f64, f64),
    sent: &AtomicBool,
    options: (u8, u8, u8),
    wheel: Option<(i32, i32)>,
) -> Result<(), String> {
    let main = MainThreadMarker::new().ok_or("native click requires the main thread")?;
    refuse_open_menu()?;
    let base: &NSView = view;
    let window = base.window().ok_or("native preview has no window")?;
    super::require_front(
        window.isVisible() && !base.isHiddenOrHasHiddenAncestor(),
        NSApplication::sharedApplication(main).isActive() && window.isKeyWindow(),
    )?;
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
    if let Some(delta) = wheel {
        return wheel::send(&hit, window_point, delta, sent);
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
    sent.store(true, Ordering::Release);
    move_pointer(base, local, &moved);
    send_events(&hit, window_point, number, options, sent)
}

fn send_events(
    hit: &NSView,
    window_point: NSPoint,
    number: isize,
    options: (u8, u8, u8),
    sent: &AtomicBool,
) -> Result<(), String> {
    let time = NSProcessInfo::processInfo().systemUptime();
    let flags = if options.2 & 1 != 0 {
        NSEventModifierFlags::Option
    } else {
        NSEventModifierFlags::empty()
    } | if options.2 & 2 != 0 {
        NSEventModifierFlags::Control
    } else {
        NSEventModifierFlags::empty()
    } | if options.2 & 4 != 0 {
        NSEventModifierFlags::Command
    } else {
        NSEventModifierFlags::empty()
    } | if options.2 & 8 != 0 {
        NSEventModifierFlags::Shift
    } else {
        NSEventModifierFlags::empty()
    };
    let (down_type, up_type) = match options.0 {
        1 => (NSEventType::OtherMouseDown, NSEventType::OtherMouseUp),
        2 => (NSEventType::RightMouseDown, NSEventType::RightMouseUp),
        _ => (NSEventType::LeftMouseDown, NSEventType::LeftMouseUp),
    };
    let button = super::macos_click_plan::explicit_button_number(options.0);
    for count in 1..=options.1 {
        let down=NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(down_type,window_point,flags,time,number,None,0,isize::from(count),1.0).ok_or("AppKit could not create mouse press")?;
        let up=NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(up_type,window_point,flags,time,number,None,0,isize::from(count),0.0).ok_or("AppKit could not create mouse release")?;
        let (down, up) = match button {
            Some(button) => (
                numbered(&down, window_point, button)?,
                numbered(&up, window_point, button)?,
            ),
            None => (down, up),
        };
        sent.store(true, Ordering::Release);
        match options.0 {
            1 => {
                hit.otherMouseDown(&down);
                hit.otherMouseUp(&up);
            }
            2 => {
                hit.rightMouseDown(&down);
                hit.rightMouseUp(&up);
            }
            _ => {
                hit.mouseDown(&down);
                hit.mouseUp(&up);
            }
        }
    }
    Ok(())
}

/// `event` with Quartz button number `button` (`macos_click_plan`), set on its Quartz copy. The copy keeps the event's
/// window and screen location; when `AppKit` converts it back without a window, the location is placed (as for the
/// wheel) so that it still names the checked window point. The result is verified before it is delivered.
fn numbered(event: &NSEvent, point: NSPoint, button: i64) -> Result<Retained<NSEvent>, String> {
    let main = MainThreadMarker::new().ok_or("native mouse input needs the main thread")?;
    let copy = event
        .CGEvent()
        .ok_or("AppKit could not copy the mouse event")?;
    CGEvent::set_integer_value_field(Some(&copy), CGEventField::MouseEventButtonNumber, button);
    let mut numbered =
        NSEvent::eventWithCGEvent(&copy).ok_or("AppKit could not convert the mouse event")?;
    if numbered.window(main).is_none() {
        let screens = NSScreen::screens(main);
        let screen = screens
            .firstObject()
            .ok_or("native mouse input found no screen")?;
        let (x, y) =
            super::macos_wheel_plan::location((point.x, point.y), screen.frame().size.height)
                .map_err(|_| "native mouse input has invalid screen geometry".to_owned())?;
        CGEvent::set_location(Some(&copy), NSPoint::new(x, y));
        numbered =
            NSEvent::eventWithCGEvent(&copy).ok_or("AppKit could not convert the mouse event")?;
    }
    let actual = numbered.locationInWindow();
    if numbered.r#type() != event.r#type()
        || numbered.buttonNumber() != isize::try_from(button).unwrap_or(-1)
        || numbered.clickCount() != event.clickCount()
        || (actual.x - point.x).abs() > 0.5
        || (actual.y - point.y).abs() > 0.5
    {
        return Err(format!(
            "the native mouse event did not keep its button, count or location (button {}, count {} of {}, at {:.1},{:.1} for {:.1},{:.1}, window {})",
            numbered.buttonNumber(),
            numbered.clickCount(),
            event.clickCount(),
            actual.x,
            actual.y,
            point.x,
            point.y,
            numbered.windowNumber()
        ));
    }
    Ok(numbered)
}

/// Hands the motion to the owners of the view's mouse-moved tracking areas at `local`, as `AppKit` does for a real
/// pointer move. `WKWebView` takes motion only from its own tracking area; `mouseMoved:` sent to the view itself is
/// dropped, so the page would never see the pointer arrive.
fn move_pointer(view: &NSView, local: NSPoint, moved: &NSEvent) {
    for area in view.trackingAreas() {
        let options = area.options();
        if !options.contains(NSTrackingAreaOptions::MouseMoved) {
            continue;
        }
        let rect = if options.contains(NSTrackingAreaOptions::InVisibleRect) {
            view.visibleRect()
        } else {
            area.rect()
        };
        if !NSPointInRect(local, rect) {
            continue;
        }
        let Some(owner) = area.owner() else {
            continue;
        };
        // SAFETY: `owner` is retained for this call; `mouseMoved:` is NSResponder's one-argument method, sent only to
        // an owner that answers it, as AppKit itself sends it to tracking-area owners on the main thread.
        unsafe {
            let answers: bool = msg_send![&*owner, respondsToSelector: sel!(mouseMoved:)];
            if answers {
                let _: () = msg_send![&*owner, mouseMoved: moved];
            }
        }
    }
}

/// Runs `work` once on the main run loop after `delay` (never earlier). Must be called on the main thread. The timer
/// runs in the run loop's common modes, so it also fires while a menu is open (`AppKit`'s event-tracking mode); in the
/// default mode alone it would wait until the person closed the menu and the caller's answer would time out.
pub fn after(delay: Duration, work: impl FnOnce() + 'static) {
    let work = RefCell::new(Some(work));
    let block = RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| {
        if let Some(work) = work.borrow_mut().take() {
            work();
        }
    });
    // SAFETY: called on the main thread (the callers' WebKit completions); the timer retains the block until it has
    // fired once, and the main run loop retains the timer while it is scheduled.
    unsafe {
        let timer =
            NSTimer::timerWithTimeInterval_repeats_block(delay.as_secs_f64(), false, &block);
        NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
    }
}
