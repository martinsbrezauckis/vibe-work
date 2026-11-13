//! Linux regression test for `reveal_main_window`'s window showing; uses the parent module, called by Cargo tests.
//! Needs an X display and GTK on the test's own thread, so it is ignored by default and runs alone:
//! `xvfb-run -a cargo test -p app --lib -- --ignored linux_reveal`.
//!
//! PREDICT: showing the window with GTK's show-all (what tao's `Window::show` does) also shows a child webview the
//! preview engine hid, so the hidden child becomes visible and mapped and both cases below fail. Showing only the
//! toplevel keeps it hidden.
use super::show_toplevel;
use gtk::prelude::*;

fn pump() {
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
}

/// One test: GTK may only be used from the thread that started it.
#[test]
#[ignore = "needs an X display: xvfb-run -a cargo test -p app --lib -- --ignored linux_reveal"]
fn linux_reveal_keeps_a_hidden_child_webview_hidden() {
    gtk::gdk::set_allowed_backends("x11");
    gtk::init().expect("GTK starts on the X display");
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_default_size(900, 600);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let app = gtk::Label::new(Some("app"));
    // Stands for a Browser page the preview engine hid (`gtk_widget_hide`) while the window was open.
    let page = gtk::Label::new(Some("hidden Browser page"));
    content.pack_start(&app, true, true, 0);
    content.pack_start(&page, true, true, 0);
    window.add(&content);
    // A Tauri window is created with show-all: every child starts visible.
    window.show_all();
    pump();
    page.hide();
    pump();

    // Brought forward while already open (a native confirmation such as Chrome's pairing approval).
    show_toplevel(&window);
    pump();
    assert!(window.is_visible() && app.is_visible());
    assert!(
        !page.is_visible() && !page.is_mapped(),
        "revealing the open window must not draw a hidden Browser page over the app"
    );

    // Opened again from the tray after closing to it.
    window.hide();
    pump();
    show_toplevel(&window);
    pump();
    assert!(window.is_visible() && app.is_visible() && app.is_mapped());
    assert!(
        !page.is_visible() && !page.is_mapped(),
        "reopening from the tray must not show a hidden Browser page"
    );
    window.close();
    pump();
}
