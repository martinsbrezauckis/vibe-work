//! `AppKit` keyboard/text input delivered only to the checked `WKWebView`'s own content responder.
//! Called on the main thread after isolated document and permission validation; never posts global events.
use super::Keyboard;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSProcessInfo, NSRange, NSString};
use objc2_web_kit::WKWebView;
use std::sync::atomic::{AtomicBool, Ordering};

/// Sends one bounded keyboard operation to this native view.
/// # Errors
/// Refuses unavailable or unfocused native views before any input.
pub fn send(
    pointer: *mut std::ffi::c_void,
    point: (f64, f64),
    input: &Keyboard,
    sent: &AtomicBool,
) -> Result<(), String> {
    let main =
        MainThreadMarker::new().ok_or("Native keyboard requires the UI thread. Try again.")?;
    // SAFETY: Tauri supplies this live WKWebView in its main-thread callback; retained for this dispatch.
    let view = unsafe { Retained::retain(pointer.cast::<WKWebView>()) }
        .ok_or("The browser view closed. Read the page again.")?;
    let base: &NSView = &view;
    let window = base
        .window()
        .ok_or("The browser window closed. Open the browser pane and try again.")?;
    if !NSApplication::sharedApplication(main).isActive()
        || !window.isKeyWindow()
        || !window.isVisible()
        || base.isHiddenOrHasHiddenAncestor()
    {
        return Err(
            "Native keyboard needs the visible active browser pane. Open it and try again.".into(),
        );
    }
    let bounds = base.bounds();
    if !(0.0..1.0).contains(&point.0)
        || !(0.0..1.0).contains(&point.1)
        || bounds.size.width < 1.0
        || bounds.size.height < 1.0
    {
        return Err("The browser's native geometry changed. Read the page and try again.".into());
    }
    let local = NSPoint::new(
        bounds.origin.x + bounds.size.width * point.0,
        bounds.origin.y
            + bounds.size.height
                * if base.isFlipped() {
                    point.1
                } else {
                    1.0 - point.1
                },
    );
    let position = base.convertPoint_toView(local, None);
    let content = window
        .contentView()
        .ok_or("The browser's native content view closed. Open the pane and try again.")?;
    // SAFETY: both views are retained on the main thread through this synchronous hit test.
    let parent = unsafe { content.superview() };
    let at = parent.as_ref().map_or(position, |parent| {
        parent.convertPoint_fromView(position, None)
    });
    if !content
        .hitTest(at)
        .is_some_and(|hit| std::ptr::eq(&raw const *hit, base) || hit.isDescendantOf(base))
    {
        return Err(
            "Another native view covers the browser target. Show the browser pane and try again."
                .into(),
        );
    }
    window.makeFirstResponder(Some(base));
    let responder = window
        .firstResponder()
        .ok_or("The browser has no keyboard responder. Focus the pane and try again.")?;
    let target = responder
        .downcast_ref::<NSView>()
        .filter(|target| std::ptr::eq(*target, base) || target.isDescendantOf(base))
        .ok_or("Keyboard focus left the browser pane. Focus it and try again.")?;
    match input {
        Keyboard::Text(text) => {
            // SAFETY: respondsToSelector checks the public NSTextInputClient entry on this live content responder.
            if !target.respondsToSelector(sel!(insertText:replacementRange:)) {
                return Err("This browser responder cannot accept native text. Choose an editable field and try again.".into());
            }
            let string = NSString::from_str(text);
            sent.store(true, Ordering::Release);
            // SAFETY: the selected public NSTextInputClient method takes NSString and NSRange. NSNotFound uses the current selection.
            unsafe {
                let _: () = msg_send![target, insertText: &*string, replacementRange: NSRange::new(isize::MAX.cast_unsigned(), 0)];
            }
        }
        Keyboard::Key(name, bits) => {
            let (characters, code) = characters(name);
            let characters = NSString::from_str(&characters);
            let flags = [
                (1, NSEventModifierFlags::Option),
                (2, NSEventModifierFlags::Control),
                (4, NSEventModifierFlags::Command),
                (8, NSEventModifierFlags::Shift),
            ]
            .into_iter()
            .filter(|(bit, _)| bits & bit != 0)
            .fold(NSEventModifierFlags::empty(), |flags, (_, bit)| flags | bit);
            let event = |kind| {
                NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind, NSPoint::new(0.0, 0.0), flags, NSProcessInfo::processInfo().systemUptime(), window.windowNumber(), None, &characters, &characters, false, code)
                .ok_or("AppKit could not create the page key. Try again.")
            };
            let down = event(NSEventType::KeyDown)?;
            let up = event(NSEventType::KeyUp)?;
            sent.store(true, Ordering::Release);
            target.keyDown(&down);
            target.keyUp(&up);
        }
    }
    Ok(())
}
fn characters(name: &str) -> (String, u16) {
    let (character, code) = match name {
        "Backspace" => ('\u{7f}', 51),
        "Tab" => ('\t', 48),
        "Enter" => ('\r', 36),
        "Escape" => ('\u{1b}', 53),
        "Space" => (' ', 49),
        "ArrowUp" => ('\u{f700}', 126),
        "ArrowDown" => ('\u{f701}', 125),
        "ArrowLeft" => ('\u{f702}', 123),
        "ArrowRight" => ('\u{f703}', 124),
        "Insert" => ('\u{f727}', 0),
        "Delete" => ('\u{f728}', 117),
        "Home" => ('\u{f729}', 115),
        "End" => ('\u{f72b}', 119),
        "PageUp" => ('\u{f72c}', 116),
        "PageDown" => ('\u{f72d}', 121),
        "Shift" => ('\0', 56),
        "Control" => ('\0', 59),
        "Alt" => ('\0', 58),
        "Meta" => ('\0', 55),
        _ => {
            return (
                name.to_owned(),
                // "a" is key code 0, the same as the fallback arm.
                match name.to_ascii_lowercase().as_str() {
                    "c" => 8,
                    "x" => 7,
                    "v" => 9,
                    "z" => 6,
                    "y" => 16,
                    _ => 0,
                },
            );
        }
    };
    (character.to_string(), code)
}
