//! GTK click feedback in the host overlay, outside WebKit's page and snapshots.
//! Depends on GTK; vw-webview calls `show` on the UI thread after agent input.

use gtk::prelude::*;
use std::time::Duration;

const PREFIX: &str = "vw-input-marker-";

/// Shows a short ring at a normalized point of the live view.
///
/// # Errors
/// Rejects invalid geometry or a view outside the host overlay.
pub fn show(view: &gtk::Widget, point: (f64, f64)) -> Result<(), String> {
    let (width, height) = (view.allocated_width(), view.allocated_height());
    let (x, y) = crate::native_click::map_point(point, (f64::from(width), f64::from(height)))?;
    if !view.is_mapped() {
        return Err("the preview is not visible".into());
    }
    let mut ancestor = view.parent();
    let overlay = loop {
        let parent = ancestor.ok_or("the preview has no host overlay")?;
        if let Ok(overlay) = parent.clone().downcast::<gtk::Overlay>() {
            break overlay;
        }
        ancestor = parent.parent();
    };
    let (left, top) = view
        .translate_coordinates(&overlay, 0, 0)
        .ok_or("the preview has no host coordinates")?;
    let name = format!("{PREFIX}{:p}", view.as_ptr());
    for old in overlay
        .children()
        .into_iter()
        .filter(|w| w.widget_name() == name)
    {
        overlay.remove(&old);
    }
    let marker = gtk::DrawingArea::new();
    // GtkOverlay's pass-through applies to its overlay window. A DrawingArea's own GDK child window
    // would still intercept the pointer, so this purely visual widget must be windowless too.
    marker.set_has_window(false);
    marker.set_widget_name(&name);
    marker.set_can_focus(false);
    marker.set_hexpand(true);
    marker.set_vexpand(true);
    // Same 28 pt view, 2 pt inset, 3 pt stroke and quarter-alpha fill as the AppKit ring.
    marker.connect_draw(move |_, context| {
        context.rectangle(
            f64::from(left),
            f64::from(top),
            f64::from(width),
            f64::from(height),
        );
        context.clip();
        context.arc(
            f64::from(left) + x,
            f64::from(top) + y,
            12.0,
            0.0,
            std::f64::consts::TAU,
        );
        context.set_source_rgba(0.0, 122.0 / 255.0, 1.0, 0.25);
        let _ = context.fill_preserve();
        context.set_line_width(3.0);
        context.set_source_rgb(0.0, 122.0 / 255.0, 1.0);
        let _ = context.stroke();
        glib::Propagation::Proceed
    });
    overlay.add_overlay(&marker);
    overlay.set_overlay_pass_through(&marker, true);
    marker.show();
    // Never reuse geometry after the native surface moved, resized, hid or closed. The callback uses weak
    // references, so neither the timer nor a marker can keep a closed page or its host alive.
    let (weak_view, weak_overlay) = (view.downgrade(), overlay.downgrade());
    marker.add_tick_callback(move |marker, _| {
        let valid =
            weak_view
                .upgrade()
                .zip(weak_overlay.upgrade())
                .is_some_and(|(view, overlay)| {
                    view.is_mapped()
                        && view.allocated_width() == width
                        && view.allocated_height() == height
                        && view.translate_coordinates(&overlay, 0, 0) == Some((left, top))
                });
        if valid {
            glib::ControlFlow::Continue
        } else {
            remove(marker);
            glib::ControlFlow::Break
        }
    });
    let weak_marker = marker.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(700), move || {
        if let Some(marker) = weak_marker.upgrade() {
            remove(&marker);
        }
    });
    Ok(())
}

fn remove(marker: &gtk::DrawingArea) {
    if let Some(parent) = marker
        .parent()
        .and_then(|p| p.downcast::<gtk::Overlay>().ok())
    {
        parent.remove(marker);
    }
}

#[cfg(test)]
#[path = "click_marker_linux_tests.rs"]
mod tests;
