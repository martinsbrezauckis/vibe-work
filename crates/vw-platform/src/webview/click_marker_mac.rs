//! The agent's click marker on macOS: a short ring at the spot the agent clicked, drawn by `AppKit` in a subview over
//! the `WKWebView`. It is part of the person's view only: no page script, DOM node or event, and it takes no input
//! (`hitTest:` answers nil, so a click at the spot still reaches the page). It removes itself after a moment.
//!
//! Depends on: objc2, objc2-app-kit (`NSView`, `NSBezierPath`, `NSColor`), objc2-web-kit. Called by: vw-webview
//! after a native click (inside Tauri's `with_webview`, on the main thread).
//! Invariant: at most one marker per webview (a new click replaces the old ring).

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSBezierPath, NSColor, NSUserInterfaceItemIdentification, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_web_kit::WKWebView;

/// Diameter of the ring in points.
const SIZE: f64 = 28.0;
/// How long the ring stays.
const SHOWN_SECONDS: f64 = 0.7;
/// The ring's view identifier (to find and replace an earlier ring).
const IDENTIFIER: &str = "VibeWorkClickMarker";

define_class!(
    /// The ring: draws itself and never takes input.
    #[unsafe(super(NSView, objc2_app_kit::NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VibeWorkClickMarkerView"]
    struct MarkerView;

    impl MarkerView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let bounds = self.bounds();
            let inset = NSRect::new(
                NSPoint::new(bounds.origin.x + 2.0, bounds.origin.y + 2.0),
                NSSize::new(bounds.size.width - 4.0, bounds.size.height - 4.0),
            );
            let ring = NSBezierPath::bezierPathWithOvalInRect(inset);
            NSColor::systemBlueColor().colorWithAlphaComponent(0.25).setFill();
            ring.fill();
            ring.setLineWidth(3.0);
            NSColor::systemBlueColor().setStroke();
            ring.stroke();
        }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }
    }
);

/// Shows the ring at `point` (0..1 of the view's width and height, from the top left) for a moment.
///
/// # Errors
/// No webview, not on the main thread, or a point outside the view.
pub fn show(webview: *mut c_void, point: (f64, f64)) -> Result<(), String> {
    show_for(
        webview,
        point,
        std::time::Duration::from_secs_f64(SHOWN_SECONDS),
    )
}

/// Paints the shared action marker for its remaining host-clock lifetime.
///
/// # Errors
/// No webview, not on the main thread, or a point outside the view.
pub fn show_for(
    webview: *mut c_void,
    point: (f64, f64),
    remaining: std::time::Duration,
) -> Result<(), String> {
    if remaining.is_zero() {
        return Ok(());
    }
    let mtm = MainThreadMarker::new().ok_or("the click marker is drawn on the main thread")?;
    if !(0.0..=1.0).contains(&point.0) || !(0.0..=1.0).contains(&point.1) {
        return Err("the click marker point is outside the view".to_owned());
    }
    // SAFETY: Tauri hands over its live `WKWebView` on the main thread inside `with_webview`.
    let Some(view) = (unsafe { webview.cast::<WKWebView>().as_ref() }) else {
        return Err("the preview webview is not available".to_owned());
    };
    remove_existing(view);
    let bounds = view.bounds();
    let x = bounds.size.width * point.0;
    // AppKit's y grows upwards unless the view is flipped.
    let y = if view.isFlipped() {
        bounds.size.height * point.1
    } else {
        bounds.size.height * (1.0 - point.1)
    };
    let frame = NSRect::new(
        NSPoint::new(x - SIZE / 2.0, y - SIZE / 2.0),
        NSSize::new(SIZE, SIZE),
    );
    let marker: Retained<MarkerView> = {
        let this = MarkerView::alloc(mtm);
        // SAFETY: NSView's designated initializer on a freshly allocated subclass instance.
        unsafe { msg_send![this, initWithFrame: frame] }
    };
    marker.setIdentifier(Some(&objc2_foundation::NSString::from_str(IDENTIFIER)));
    view.addSubview(&marker);
    // SAFETY: `removeFromSuperview` takes no argument; the run loop retains the target until the delayed call ran.
    unsafe {
        let _: () = msg_send![
            &*marker,
            performSelector: sel!(removeFromSuperview),
            withObject: std::ptr::null::<AnyObject>(),
            afterDelay: remaining.as_secs_f64().min(SHOWN_SECONDS)
        ];
    }
    Ok(())
}

/// The ring currently shown over `webview`, as (x, y, width, height) in the view's coordinates (tests).
#[must_use]
pub fn shown(webview: *mut c_void) -> Option<(f64, f64, f64, f64)> {
    // SAFETY: as in `show`.
    let view = unsafe { webview.cast::<WKWebView>().as_ref() }?;
    marker_of(view).map(|m| {
        let f = m.frame();
        (f.origin.x, f.origin.y, f.size.width, f.size.height)
    })
}

/// True when a hit test at the ring's centre reaches a view other than the ring (the page still gets the click).
#[must_use]
pub fn passes_input(webview: *mut c_void) -> bool {
    // SAFETY: as in `show`.
    let Some(view) = (unsafe { webview.cast::<WKWebView>().as_ref() }) else {
        return false;
    };
    let Some(marker) = marker_of(view) else {
        return false;
    };
    let f = marker.frame();
    let centre = NSPoint::new(
        f.origin.x + f.size.width / 2.0,
        f.origin.y + f.size.height / 2.0,
    );
    // `hitTest:` takes a point in the superview's coordinates.
    let Some(parent) = (unsafe { view.superview() }) else {
        return false;
    };
    let in_parent = view.convertPoint_toView(centre, Some(&parent));
    view.hitTest(in_parent).is_some_and(|hit| {
        !std::ptr::eq(
            Retained::as_ptr(&hit).cast::<()>(),
            Retained::as_ptr(&marker).cast::<()>(),
        )
    })
}

fn marker_of(view: &WKWebView) -> Option<Retained<NSView>> {
    view.subviews().iter().find(|sub| {
        sub.identifier()
            .is_some_and(|id| id.to_string() == IDENTIFIER)
    })
}

fn remove_existing(view: &WKWebView) {
    while let Some(old) = marker_of(view) {
        old.removeFromSuperview();
    }
}
