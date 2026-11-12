//! Public Quartz wheel construction with direct delivery to the already checked `WKWebView` hit view.
//! Called only by the macOS native input adapter after foreground, visibility and geometry checks.
//! No event tap, global posting, private `AppKit` selector or persistent gesture is used.

use std::sync::atomic::{AtomicBool, Ordering};

use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSEventType, NSScreen, NSView};
use objc2_core_graphics::{CGEvent, CGScrollEventUnit};
use objc2_foundation::NSPoint;

pub(super) fn send(
    hit: &NSView,
    point: NSPoint,
    delta: (i32, i32),
    sent: &AtomicBool,
) -> Result<(), String> {
    let refusal = super::super::macos_wheel_plan::refusal;
    let main = MainThreadMarker::new()
        .ok_or_else(|| refusal("Native wheel delivery requires the main thread"))?;
    let screens = NSScreen::screens(main);
    let screen = screens
        .firstObject()
        .ok_or_else(|| refusal("The native wheel has no screen"))?;
    let (x, y) =
        super::super::macos_wheel_plan::location((point.x, point.y), screen.frame().size.height)?;
    // This is CGEventCreateScrollWheelEvent2, with pixel deltas and no held phase/momentum.
    let cg =
        CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Pixel, 2, delta.0, delta.1, 0)
            .ok_or_else(|| refusal("Quartz could not create the wheel event"))?;
    CGEvent::set_location(Some(&cg), NSPoint::new(x, y));
    let event = NSEvent::eventWithCGEvent(&cg)
        .ok_or_else(|| refusal("AppKit could not convert the wheel event"))?;
    // Windowless conversion uses the first screen's flipped coordinates. Verify that the public
    // conversion retained our window-local point before delivering directly to the hit view.
    let actual = event.locationInWindow();
    if event.r#type() != NSEventType::ScrollWheel
        || event.window(main).is_some()
        || (actual.x - point.x).abs() > 0.5
        || (actual.y - point.y).abs() > 0.5
    {
        return Err(refusal(
            "The native wheel event did not retain the checked location",
        ));
    }
    sent.store(true, Ordering::Release);
    hit.scrollWheel(&event);
    Ok(())
}
