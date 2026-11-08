//! Native marker acceptance on an isolated GTK display. Exercises the production drawing API and lifecycle.
//! Called by cargo test with Xvfb; GTK, not a mocked renderer, owns the overlay and allocation.

use super::*;
use gdk::prelude::WindowExtManual;
use std::time::{Duration, Instant};

fn pump(duration: Duration) {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "requires an isolated display: xvfb-run cargo test -p vw-platform --features webview click_marker -- --ignored"]
fn marker_is_visible_replaced_passes_input_and_expires() {
    gtk::init().unwrap();
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    let overlay = gtk::Overlay::new();
    let layer = gtk::Fixed::new();
    let view = gtk::DrawingArea::new();
    view.set_size_request(400, 240);
    layer.put(&view, 60, 40);
    overlay.add(&layer);
    window.add(&overlay);
    window.set_default_size(600, 400);
    window.show_all();
    pump(Duration::from_millis(100));

    show(view.upcast_ref(), (0.5, 0.25)).unwrap();
    pump(Duration::from_millis(40));
    let markers: Vec<_> = overlay
        .children()
        .into_iter()
        .filter(|w| w != &layer)
        .collect();
    assert_eq!(
        markers.len(),
        1,
        "a successful agent input must draw one host marker"
    );
    assert!(overlay.is_overlay_pass_through(&markers[0]));
    assert!(!markers[0].can_focus());
    let pixel = window
        .window()
        .unwrap()
        .pixbuf(272, 100, 1, 1)
        .unwrap()
        .read_pixel_bytes();
    assert!(
        pixel[2] > 200 && pixel[0] < 40,
        "the ring must actually paint blue at its mapped position: {pixel:?}"
    );
    let first = markers[0].clone();
    pump(Duration::from_millis(400));
    show(view.upcast_ref(), (0.1, 0.9)).unwrap();
    assert!(
        first.parent().is_none(),
        "new input replaces the previous marker"
    );
    assert_eq!(overlay.children().len(), 2);
    assert!(show(view.upcast_ref(), (f64::NAN, 0.5)).is_err());
    assert!(show(view.upcast_ref(), (1.0, 0.5)).is_err());
    pump(Duration::from_millis(350));
    assert_eq!(
        overlay.children().len(),
        2,
        "an older timer cannot remove the replacement ring"
    );
    pump(Duration::from_millis(400));
    assert_eq!(overlay.children().len(), 1, "the ring expires after 700 ms");
    show(view.upcast_ref(), (0.5, 0.5)).unwrap();
    view.set_size_request(450, 240);
    pump(Duration::from_millis(80));
    assert_eq!(
        overlay.children().len(),
        1,
        "a resized surface invalidates the old point"
    );
    show(view.upcast_ref(), (0.5, 0.5)).unwrap();
    view.hide();
    pump(Duration::from_millis(80));
    assert_eq!(
        overlay.children().len(),
        1,
        "a hidden surface cannot retain feedback"
    );
    view.show();
    pump(Duration::from_millis(40));
    show_for(view.upcast_ref(), (0.5, 0.5), Duration::ZERO).unwrap();
    assert_eq!(
        overlay.children().len(),
        1,
        "expired feedback must not paint"
    );
    show_for(view.upcast_ref(), (0.5, 0.5), Duration::from_millis(40)).unwrap();
    assert_eq!(overlay.children().len(), 2);
    pump(Duration::from_millis(100));
    assert_eq!(
        overlay.children().len(),
        1,
        "the native timer must keep the remaining lifetime"
    );
    window.close();
}
