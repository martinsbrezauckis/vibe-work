//! Native full-screen geometry regression. Called by cargo test on an isolated Xvfb display.
//! Uses `WebKitGTK` in the production preview layout; no page or window-manager mocks.

use super::*;

#[test]
#[ignore = "requires an isolated display: xvfb-run cargo test -p vw-webview full_screen_fills -- --ignored"]
fn full_screen_fills_the_layer_and_restores_the_panel() {
    gtk::init().unwrap();
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_default_size(900, 600);
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let main = webkit2gtk::WebView::new();
    let page = webkit2gtk::WebView::new();
    vbox.pack_start(&main, true, true, 0);
    vbox.pack_start(&page, true, true, 0);
    window.add(&vbox);
    window.show_all();
    show_at(&page, (120, 60, 400, 240)).unwrap();
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
    let layer = page.parent().unwrap().downcast::<gtk::Fixed>().unwrap();
    page_full_screen(&page, true);
    allocate_exact(&layer, page.upcast_ref());
    let a = page.allocation();
    let bounds = layer.allocation();
    assert_eq!(
        (a.x(), a.y(), a.width(), a.height()),
        (bounds.x(), bounds.y(), bounds.width(), bounds.height()),
        "the person's full-screen page must fill the window, without the normal app panels"
    );
    // App ResizeObserver updates must not put the page back into a panel during full screen.
    show_at(&page, (100, 50, 450, 270)).unwrap();
    assert_eq!(page.allocation().width(), layer.allocation().width());
    page_full_screen(&page, false);
    let a = page.allocation();
    assert_eq!(
        (a.x(), a.y(), a.width(), a.height()),
        (bounds.x() + 100, bounds.y() + 50, 450, 270)
    );
    window.close();
}
