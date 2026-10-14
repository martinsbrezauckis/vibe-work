//! Places preview webviews over the main webview on `WebKitGTK`. Ported from ShellX
//! `src-tauri/src/shellx_browser_native_layout_linux.rs:1-130` @ e38c2b5c.
//!
//! Why: with the `unstable` multiwebview feature, Tauri packs every webview of a window into one `GtkBox`, which
//! divides the space between them and ignores the bounds given to a child (ShellX scar, still true for tauri
//! 2.12.1). So the main webview is moved into a `GtkOverlay` (it keeps the whole window) and the previews go into
//! a named `GtkFixed` layer on top of it, where explicit positions and sizes work.
//!
//! Depends on: gtk, webkit2gtk (both from Tauri's own Linux stack). Called by: `engine` (inside `with_webview`, on
//! the GTK main thread).
//! Invariants:
//! - Only the widgets named here are touched; any other layout found is an error, never "fixed up" by guessing.
//! - Bounds are checked (`vw_preview::Rect::checked`) before they reach GTK.

use gtk::prelude::*;

const OVERLAY_NAME: &str = "vw-preview-overlay";
const LAYER_NAME: &str = "vw-preview-layer";

/// Integer pixel geometry for GTK (`x`, `y`, `width`, `height`).
pub(crate) type Geometry = (i32, i32, i32, i32);

// The native-paint investigation needs GTK's *actual* geometry after its parent has laid out.
// This is opt-in and records widget geometry only: never the page, URL or account state.
fn trace_enabled() -> bool {
    std::env::var("VW_NATIVE_LAYOUT_TRACE").as_deref() == Ok("1")
}

fn trace_layout(stage: &'static str, engine: &gtk::Widget, scheduled: Option<Geometry>) {
    if !trace_enabled() {
        return;
    }
    let Some(layer) = engine
        .parent()
        .and_then(|p| p.downcast::<gtk::Fixed>().ok())
    else {
        return;
    };
    if layer.widget_name() != LAYER_NAME {
        return;
    }
    let layer_allocation = layer.allocation();
    let allocation = engine.allocation();
    let clip = engine.clip();
    let (minimum, natural) = engine.preferred_size();
    let request = engine.size_request();
    let fixed_position = (layer.child_x(engine), layer.child_y(engine));
    let overlay = layer.parent();
    let main = overlay
        .as_ref()
        .and_then(|parent| parent.clone().downcast::<gtk::Overlay>().ok())
        .and_then(|parent| parent.child());
    let relative_main = main
        .as_ref()
        .and_then(|main| engine.translate_coordinates(main, 0, 0));
    let relative_overlay = overlay
        .as_ref()
        .and_then(|overlay| engine.translate_coordinates(overlay, 0, 0));
    let native_window = engine.window();
    let main_window = main.as_ref().and_then(gtk::prelude::WidgetExt::window);
    let native_root = native_window
        .as_ref()
        .map(|window| window.root_coords(0, 0));
    let main_root = main_window.as_ref().map(|window| window.root_coords(0, 0));
    let native_relative_main = native_root
        .zip(main_root)
        .map(|((x, y), (mx, my))| (x - mx, y - my));
    let native_geometry = native_window.as_ref().map(gtk::gdk::Window::geometry);
    let native_position = native_window.as_ref().map(gtk::gdk::Window::position);
    let native_visible = native_window.as_ref().map(gtk::gdk::Window::is_visible);
    let native_viewable = native_window.as_ref().map(gtk::gdk::Window::is_viewable);
    // A WebKit widget may have its own GDK child windows; bound the diagnostic inventory.
    let native_children: Vec<_> = native_window
        .as_ref()
        .filter(|_| engine.has_window())
        .map(|window| {
            window
                .children()
                .into_iter()
                .take(8)
                .map(|child| {
                    (
                        child.as_ptr(),
                        child.geometry(),
                        child.root_coords(0, 0),
                        child.is_visible(),
                        child.is_viewable(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    tracing::info!(
        stage,
        widget = ?engine.as_ptr(),
        ?scheduled,
        ?fixed_position,
        request = ?request,
        preferred_min = ?(minimum.width, minimum.height),
        preferred_nat = ?(natural.width, natural.height),
        fixed_allocation = ?(layer_allocation.x(), layer_allocation.y(), layer_allocation.width(), layer_allocation.height()),
        allocation = ?(allocation.x(), allocation.y(), allocation.width(), allocation.height()),
        clip = ?(clip.x(), clip.y(), clip.width(), clip.height()),
        ?relative_main,
        ?relative_overlay,
        native_has_window = engine.has_window(),
        native_window = ?native_window.as_ref().map(glib::object::ObjectType::as_ptr),
        main_window = ?main_window.as_ref().map(glib::object::ObjectType::as_ptr),
        ?native_geometry,
        ?native_position,
        ?native_root,
        ?native_relative_main,
        ?native_visible,
        ?native_viewable,
        ?native_children,
        visible = engine.is_visible(),
        child_visible = engine.is_child_visible(),
        mapped = engine.is_mapped(),
        scale = engine.scale_factor(),
        "native layout trace"
    );
}

/// Records native GTK allocation around a Browser chrome handoff (no page content or URL).
pub(crate) fn trace_overlay(engine: &webkit2gtk::WebView, hidden: bool, after: bool) {
    let stage = match (hidden, after) {
        (true, false) => "browser-overlay-before-hide",
        (true, true) => "browser-overlay-after-hide",
        (false, false) => "browser-overlay-before-restore",
        (false, true) => "browser-overlay-after-restore",
    };
    trace_layout(stage, engine.upcast_ref(), None);
}

fn trace_after_idle(engine: &webkit2gtk::WebView, scheduled: Geometry) {
    if !trace_enabled() {
        return;
    }
    let weak = engine.downgrade();
    gtk::glib::idle_add_local_once(move || {
        if let Some(engine) = weak.upgrade() {
            trace_layout("idle-after-place", engine.upcast_ref(), Some(scheduled));
        }
    });
}

// GtkFixed normally reallocates a child from its preferred minimum. A hidden child can still carry the minimum
// from its previous visible viewport, so use the explicit request after the parent's allocator has run.
fn allocate_exact(layer: &gtk::Fixed, child: &gtk::Widget) {
    let (width, height) = child.size_request();
    if !child.is_visible() || width <= 0 || height <= 0 {
        return;
    }
    let parent = layer.allocation();
    let x = parent.x().saturating_add(layer.child_x(child));
    let y = parent.y().saturating_add(layer.child_y(child));
    child.size_allocate(&gtk::Allocation::new(x, y, width, height));
}

/// Converts checked logical bounds to GTK integers.
pub(crate) fn geometry(bounds: vw_preview::Rect) -> Result<Geometry, String> {
    let bounds = bounds.checked()?;
    let int = |v: f64| -> i32 {
        // Checked above: finite and within 0..=MAX_COORD (100,000), so the rounded value fits an i32.
        #[allow(clippy::cast_possible_truncation)]
        let value = v.round() as i32;
        value
    };
    Ok((
        int(bounds.x),
        int(bounds.y),
        int(bounds.width),
        int(bounds.height),
    ))
}

/// Puts `engine` into the preview layer (creating the overlay on first use) at `geometry`.
pub(crate) fn place(engine: &webkit2gtk::WebView, geometry: Geometry) -> Result<(), String> {
    let (x, y, width, height) = geometry;
    let parent = engine
        .parent()
        .ok_or_else(|| "the preview has no GTK parent".to_owned())?;
    let layer = if let Ok(fixed) = parent.clone().downcast::<gtk::Fixed>() {
        if fixed.widget_name() != LAYER_NAME {
            return Err("the preview sits in a GTK layer Vibe Work did not create".to_owned());
        }
        fixed
    } else {
        let vbox = parent
            .downcast::<gtk::Box>()
            .map_err(|_| "the preview's GTK parent is not Tauri's box".to_owned())?;
        let layer = layer_in(&vbox, engine)?;
        vbox.remove(engine);
        layer.put(engine, x, y);
        layer
    };
    trace_layout("before-place", engine.upcast_ref(), Some(geometry));
    // A GtkWidget that is not GTK-visible ignores size_allocate. Keep it unmapped while making it visible,
    // including after reparenting (which resets child-visible), then give it the new size before mapping it.
    engine.set_child_visible(false);
    engine.set_size_request(width, height);
    layer.move_(engine, x, y);
    engine.show();
    allocate_exact(&layer, engine.upcast_ref());
    trace_layout("immediate-after-place", engine.upcast_ref(), Some(geometry));
    trace_after_idle(engine, geometry);
    Ok(())
}

/// Shows `engine` at `geometry`: placed first, then mapped (never mapped for a frame at its old spot or
/// size). `place` makes even a previously hidden widget GTK-visible while unmapped; only this function then maps it.
pub(crate) fn show_at(engine: &webkit2gtk::WebView, geometry: Geometry) -> Result<(), String> {
    place(engine, geometry)?;
    engine.set_child_visible(true);
    trace_layout("after-child-visible", engine.upcast_ref(), Some(geometry));
    Ok(())
}

/// Parks `engine` unmapped at `width` x `height`. GTK 3 ignores `size_allocate` on a hidden widget
/// (`gtk_widget_size_allocate_with_baseline` returns early unless the widget is visible), so a hidden webview would
/// keep whatever size it had (the engine's old 16 px park: a ~136 px wide page). Instead the widget stays VISIBLE but
/// not CHILD-visible (`gtk_widget_set_child_visible(FALSE)`, how `GtkNotebook` keeps its hidden pages allocated): it
/// is never mapped, so nothing of it is drawn and the page is `visibilityState: hidden`, while every allocation
/// reaches `WebKitGTK` and the page lays out at that viewport. Order matters, all in this one UI-thread task (GTK
/// draws in a later frame): first into the preview layer (`place`; moving it there resets the child-visible flag,
/// GTK's unparent sets it back to TRUE), then unmapped, then shown (visible, still unmapped) and allocated. Placed at
/// the layer's origin: the spot does not matter while unmapped.
pub(crate) fn park(engine: &webkit2gtk::WebView, width: i32, height: i32) -> Result<(), String> {
    place(engine, (0, 0, width, height))?;
    trace_layout(
        "after-park",
        engine.upcast_ref(),
        Some((0, 0, width, height)),
    );
    Ok(())
}

/// Parks a Browser child at ShellX's background sentinel while keeping its current viewport size.
/// The child is unmapped before moving, so GTK never paints it over the chrome.
pub(crate) fn park_browser_overlay(
    engine: &webkit2gtk::WebView,
    width: i32,
    height: i32,
) -> Result<(), String> {
    let geometry = (
        crate::platform::BROWSER_PARKED_OFFSET,
        crate::platform::BROWSER_PARKED_OFFSET,
        width,
        height,
    );
    place(engine, geometry)?;
    trace_layout(
        "browser-overlay-after-park",
        engine.upcast_ref(),
        Some(geometry),
    );
    Ok(())
}

/// The preview layer of Tauri's box: the existing one, or a new overlay around the main webview.
fn layer_in(vbox: &gtk::Box, engine: &webkit2gtk::WebView) -> Result<gtk::Fixed, String> {
    let children = vbox.children();
    let overlays: Vec<gtk::Widget> = children
        .iter()
        .filter(|child| child.widget_name() == OVERLAY_NAME)
        .cloned()
        .collect();
    match overlays.as_slice() {
        [overlay] => {
            let overlay = overlay
                .clone()
                .downcast::<gtk::Overlay>()
                .map_err(|_| "the preview overlay has the wrong GTK type".to_owned())?;
            let layers: Vec<gtk::Widget> = overlay
                .children()
                .into_iter()
                .filter(|child| child.widget_name() == LAYER_NAME)
                .collect();
            match layers.as_slice() {
                [layer] => layer
                    .clone()
                    .downcast::<gtk::Fixed>()
                    .map_err(|_| "the preview layer has the wrong GTK type".to_owned()),
                _ => Err("the preview overlay has no unique layer".to_owned()),
            }
        }
        [] => {
            let main: Vec<(usize, &gtk::Widget)> = children
                .iter()
                .enumerate()
                .filter(|(_, child)| {
                    child.is::<webkit2gtk::WebView>()
                        && *child != engine.upcast_ref::<gtk::Widget>()
                })
                .collect();
            let [(position, main)] = main.as_slice() else {
                return Err("Tauri's box has no unique main webview".to_owned());
            };
            let overlay = gtk::Overlay::new();
            overlay.set_widget_name(OVERLAY_NAME);
            let fixed = gtk::Fixed::new();
            fixed.set_widget_name(LAYER_NAME);
            fixed.set_hexpand(true);
            fixed.set_vexpand(true);
            // GTK emits this after GtkFixed's default allocator has assigned its children. Restore each explicit
            // request on every later window resize and tab transition, including GTK-visible parked views.
            fixed.connect_size_allocate(|fixed, _| {
                for child in fixed.children() {
                    if child.is::<webkit2gtk::WebView>() {
                        trace_layout("before-exact-allocation", &child, None);
                        allocate_exact(fixed, &child);
                        trace_layout("after-fixed-allocation", &child, None);
                    }
                }
            });
            // Moving the main webview out of the box drops the window's focus widget (it is unparented for a
            // moment): give it back, so creating the first preview never takes the focus from the app's UI
            // main webview.
            let focused = main.is_focus();
            vbox.remove(*main);
            overlay.add(*main);
            overlay.add_overlay(&fixed);
            // GtkOverlay gives even this windowless layer a full-window input window. Pass through its empty
            // area to the main webview; each native WebView's own child window still receives page input.
            overlay.set_overlay_pass_through(&fixed, true);
            vbox.pack_start(&overlay, true, true, 0);
            vbox.reorder_child(&overlay, i32::try_from(*position).unwrap_or(0));
            fixed.show();
            overlay.show();
            if focused {
                main.grab_focus();
            }
            Ok(fixed)
        }
        _ => Err("Tauri's box has several preview overlays".to_owned()),
    }
}
