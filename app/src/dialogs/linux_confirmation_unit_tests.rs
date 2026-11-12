//! Regression tests for `linux_confirmation.rs`; uses the parent module and is called by Cargo tests.
//! The answer mapping runs everywhere. The window tests need an X display and Tauri's mock
//! runtime (its main-thread tasks run at once on the calling thread, so the GTK window lives on
//! the test's thread): `xvfb-run -a cargo test -p app --lib -- --ignored linux_confirmation`.
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use super::*;
use gtk::prelude::*;

fn labels(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_request_without_one_to_three_buttons_shows_nothing_and_declines() {
    // Refused before anything reaches GTK, so no display is needed.
    let app = tauri::test::mock_app();
    for count in [0, 4] {
        let names = vec!["Continue".to_owned(); count];
        let answer = poll_once(&mut Box::pin(show_dialog(
            app.handle(),
            "text".to_owned(),
            &names,
            true,
            None,
        )));
        assert_eq!(answer, Poll::Ready(None), "{count} buttons");
    }
    assert!(
        !gtk::is_initialized(),
        "invalid buttons never initialize GTK"
    );
}

fn poll_once<F: Future + ?Sized>(future: &mut Pin<Box<F>>) -> Poll<F::Output> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}

/// Runs GTK's pending events (the window is mapped on the X display, the answer is delivered).
fn pump() {
    while gtk::events_pending() {
        gtk::main_iteration();
    }
}

/// Runs GTK's events for `ms` (a button pressed from the keyboard answers after its short press animation).
fn pump_for(ms: u64) {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    while std::time::Instant::now() < until {
        pump();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// The one open confirmation window.
fn only_window() -> gtk::MessageDialog {
    let mut windows: Vec<_> = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|window| window.downcast::<gtk::MessageDialog>().ok())
        .filter(gtk::prelude::WidgetExt::is_visible)
        .collect();
    assert_eq!(windows.len(), 1, "exactly one visible confirmation window");
    windows.remove(0)
}

fn button_labels(dialog: &gtk::MessageDialog) -> Vec<String> {
    let area = dialog
        .widget_for_response(gtk::ResponseType::Other(1))
        .and_then(|button| button.parent())
        .and_then(|parent| parent.downcast::<gtk::Container>().ok())
        .expect("the dialog's button area");
    area.children()
        .into_iter()
        .filter_map(|child| child.downcast::<gtk::Button>().ok())
        .filter_map(|button| button.label().map(|label| label.to_string()))
        .collect()
}

/// The declining (last) button is the default and has the focus: Return declines, with two buttons and with
/// three. The question heads, who asks comes below.
fn the_declining_button_is_the_default(app: &tauri::App<tauri::test::MockRuntime>, text: &str) {
    for names in [
        labels(&["Continue", "Cancel"]),
        labels(&["Allow once", "Always", "Deny"]),
    ] {
        let asked = format!("{text}\n\n{}\n\nCommand:\nrm -r notes", vw_host::AGENT_LINE);
        let mut waiting = Box::pin(show_dialog(app.handle(), asked, &names, true, None));
        assert_eq!(poll_once(&mut waiting), Poll::Pending);
        pump();
        let dialog = only_window();
        assert_eq!(dialog.text().as_deref(), Some(text));
        assert_eq!(
            dialog.secondary_text().as_deref(),
            Some(format!("{}\n\nCommand:\nrm -r notes", vw_host::AGENT_LINE).as_str())
        );
        let declining = dialog
            .widget_for_response(gtk::ResponseType::Other(
                u16::try_from(names.len()).unwrap(),
            ))
            .expect("the declining button");
        assert!(
            declining.has_default(),
            "{names:?}: the declining button is the default"
        );
        assert!(
            declining.is_focus(),
            "{names:?}: the declining button has the focus"
        );
        // GtkWindow binds Return to "activate-focus" (Xvfb without a window manager gives the window no
        // keyboard focus, so the key itself is sent by the e2e scripts through xdotool).
        dialog.emit_activate_focus();
        pump_for(500);
        assert_eq!(
            poll_once(&mut waiting),
            Poll::Ready(names.last().cloned()),
            "{names:?}: Return answers the declining button"
        );
    }
}

/// Real keys through the X display (as the e2e scripts send them): each Shift+Tab goes one button to the left,
/// Return presses the focused one, a plain Return declines. Needs xdotool.
fn real_keys_reach_the_buttons(app: &tauri::App<tauri::test::MockRuntime>, text: &str) {
    let three = labels(&["Allow once", "Always", "Deny"]);
    let mut waiting = Box::pin(show_dialog(
        app.handle(),
        text.to_owned(),
        &three,
        true,
        None,
    ));
    assert_eq!(poll_once(&mut waiting), Poll::Pending);
    pump_for(300);
    let dialog = only_window();
    let xdotool = |args: &[&str]| {
        let out = std::process::Command::new("xdotool")
            .args(args)
            .output()
            .expect("xdotool runs (the window test sends real keys with it)");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let found = xdotool(&["search", "--name", vw_host::DIALOG_TITLE]);
    let id = found
        .lines()
        .next()
        .expect("the dialog's X window")
        .to_owned();
    xdotool(&["windowfocus", "--sync", &id]);
    pump_for(200);
    let focused = |n: u16| {
        dialog
            .widget_for_response(gtk::ResponseType::Other(n))
            .is_some_and(|button| button.is_focus())
    };
    assert!(focused(3), "the focus starts on Deny");
    xdotool(&["key", "shift+Tab"]);
    pump_for(300);
    assert!(focused(2), "Shift+Tab: the second button");
    xdotool(&["key", "shift+Tab"]);
    pump_for(300);
    assert!(focused(1), "Shift+Tab again: the first button");
    xdotool(&["key", "Return"]);
    pump_for(600);
    assert_eq!(
        poll_once(&mut waiting),
        Poll::Ready(Some("Allow once".to_owned()))
    );
    let mut waiting = Box::pin(show_dialog(
        app.handle(),
        text.to_owned(),
        &three,
        true,
        None,
    ));
    assert_eq!(poll_once(&mut waiting), Poll::Pending);
    pump_for(300);
    let found = xdotool(&["search", "--name", vw_host::DIALOG_TITLE]);
    let id = found
        .lines()
        .next()
        .expect("the dialog's X window")
        .to_owned();
    xdotool(&["windowfocus", "--sync", &id]);
    pump_for(200);
    xdotool(&["key", "Return"]);
    pump_for(600);
    assert_eq!(
        poll_once(&mut waiting),
        Poll::Ready(Some("Deny".to_owned())),
        "a real Return declines"
    );
}

/// Orca's and a voice tool's way: the probe finds the window in the accessibility tree, prints what it reads
/// (role, text in order, buttons with their states) and presses one button; the press answers that button.
/// Every button of every layout the app shows (the vault's consent, an MCP server's key, a plain
/// confirmation, the new-chat restore) is pressed once, and the names in the tree are the buttons' labels.
fn screen_reader_presses(app: &tauri::App<tauri::test::MockRuntime>, text: &str, probe: &str) {
    let layouts = [
        labels(&["Allow once", "Always", "Deny"]),
        labels(&[
            "Always allow",
            "Allow until Vibe Work closes",
            "Don't allow",
        ]),
        labels(&["Continue", "Cancel"]),
        labels(&["Keep", "Pick again"]),
        labels(&[vw_vocabulary::texts::CONNECTOR_CONNECT, "Not now"]),
    ];
    for (names, label) in layouts
        .iter()
        .flat_map(|names| names.iter().map(move |label| (names, label.as_str())))
    {
        let declining = names.last().expect("a layout has buttons");
        let asked = format!("{text}\n\n{}", vw_host::AGENT_LINE);
        let mut waiting = Box::pin(show_dialog(app.handle(), asked, names, true, None));
        assert_eq!(poll_once(&mut waiting), Poll::Pending);
        pump_for(300);
        // What the buttons show, read from the widgets themselves.
        let mut shown = button_labels(&only_window());
        // A desktop gives the new window the keyboard focus (Xvfb has no window manager to do it).
        let found = std::process::Command::new("xdotool")
            .args(["search", "--name", vw_host::DIALOG_TITLE])
            .output()
            .expect("xdotool runs");
        let id = String::from_utf8_lossy(&found.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        let _ = std::process::Command::new("xdotool")
            .args(["windowfocus", "--sync", &id])
            .status();
        pump_for(200);
        let mut child = std::process::Command::new("python3")
            .args(["-I", probe, label])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("python3 runs the accessibility probe");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let status = loop {
            pump();
            if let Some(status) = child.try_wait().expect("the probe runs") {
                break status;
            }
            assert!(std::time::Instant::now() < until, "the probe finished");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let mut said = String::new();
        std::io::Read::read_to_string(&mut child.stdout.take().unwrap(), &mut said).unwrap();
        eprintln!("{said}");
        assert!(status.success(), "{label}: {said}");
        assert!(said.contains("WINDOW role=alert"), "{said}");
        let first_text = said
            .lines()
            .find(|l| l.starts_with("TEXT "))
            .unwrap_or_default();
        assert_eq!(
            first_text,
            format!("TEXT {text}"),
            "the question is read first"
        );
        assert!(
            said.contains(&format!("BUTTON {declining} | focused default")),
            "{said}"
        );
        let mut read: Vec<String> = said
            .lines()
            .filter_map(|l| l.strip_prefix("BUTTON "))
            .map(|l| l.split(" | ").next().unwrap_or_default().to_owned())
            .collect();
        shown.sort_unstable();
        read.sort_unstable();
        assert_eq!(read, shown, "the tree names each button by its label");
        pump_for(600);
        assert_eq!(
            poll_once(&mut waiting),
            Poll::Ready(Some(label.to_owned())),
            "{label}"
        );
    }
}

fn browser_pairing_dialogs(app: &tauri::App<tauri::test::MockRuntime>) {
    let settings = gtk::Settings::default().unwrap();
    settings.set_gtk_enable_animations(false);
    for dark in [false, true] {
        settings.set_gtk_theme_name(Some("Adwaita"));
        settings.set_gtk_application_prefer_dark_theme(dark);
        pump_for(500);
        for browser in ["Chrome", "Edge"] {
            let request = vw_gate::ConfirmationRequest {
                requester: vw_gate::Requester::Core,
                method: vw_core_api::BROWSER_PAIRING_DIALOG_METHOD.into(),
                params: serde_json::json!({}),
                summary: format!(
                    "Connect {browser} to Vibe Work?\n\n{browser} needs your approval. Choose Connect to allow this connection."
                ),
                always: None,
            };
            let names = vw_host::dialog_buttons(&request);
            let mut waiting = Box::pin(show_dialog(
                app.handle(),
                vw_host::dialog_text(&request).unwrap(),
                &names,
                true,
                None,
            ));
            assert_eq!(poll_once(&mut waiting), Poll::Pending);
            pump_for(300);
            let dialog = only_window();
            assert_eq!(
                dialog.text().as_deref(),
                Some(format!("Connect {browser} to Vibe Work?").as_str())
            );
            assert_eq!(
                button_labels(&dialog),
                [vw_vocabulary::texts::CONNECTOR_CONNECT, "Not now"]
            );
            if let Ok(dir) = std::env::var("VW_PAIRING_SCREENSHOTS") {
                dialog.queue_draw();
                pump_for(1000);
                gtk::gdk::Display::default().unwrap().sync();
                let theme = if dark { "dark" } else { "light" };
                let output = std::path::Path::new(&dir).join(format!("{browser}-{theme}.png"));
                assert!(
                    std::process::Command::new("import")
                        .args(["-window", "root"])
                        .arg(output)
                        .status()
                        .unwrap()
                        .success()
                );
            }
            dialog.response(gtk::ResponseType::Other(2));
            pump();
            assert_eq!(poll_once(&mut waiting), Poll::Ready(Some("Not now".into())));
        }
    }
    settings.set_gtk_application_prefer_dark_theme(false);
}

/// One test, because GTK may only be used from the thread that started it.
#[test]
#[ignore = "needs an X display: run with xvfb-run -a cargo test -p app --lib -- --ignored linux_confirmation"]
fn the_window_shows_answers_and_closes_when_its_request_is_dropped() {
    // The X display xvfb-run made, even where a desktop's Wayland socket exists (GTK would pick that first).
    gtk::gdk::set_allowed_backends("x11");
    gtk::init().expect("GTK starts on the X display");
    let app = tauri::test::mock_app();
    let names = labels(&["Continue", "Cancel"]);
    let text = "Delete the session \"Notes\"?".to_owned();

    // A pressed button answers its label, and the window goes away.
    let mut waiting = Box::pin(show_dialog(app.handle(), text.clone(), &names, true, None));
    assert_eq!(poll_once(&mut waiting), Poll::Pending, "waits for a button");
    pump();
    let dialog = only_window();
    assert!(dialog.is_visible(), "the window is shown");
    assert!(dialog.is_modal());
    assert_eq!(
        dialog.title().as_deref(),
        Some("Vibe Work asks for your confirmation")
    );
    // The question is the primary text (read first by Orca); nothing else to say here.
    assert_eq!(dialog.text().as_deref(), Some(text.as_str()));
    assert_eq!(dialog.secondary_text(), None);
    assert_eq!(button_labels(&dialog), names, "buttons in the given order");
    dialog.response(gtk::ResponseType::Other(1));
    pump();
    assert_eq!(
        poll_once(&mut waiting),
        Poll::Ready(Some("Continue".to_owned()))
    );
    assert!(!dialog.is_visible(), "the answered window is gone");
    assert!(
        !gtk::Window::list_toplevels()
            .iter()
            .any(gtk::prelude::WidgetExt::is_visible)
    );

    the_declining_button_is_the_default(&app, &text);
    real_keys_reach_the_buttons(&app, &text);
    // A screen reader's press through the accessibility tree: only where a script provides the probe and an
    // accessibility bus (the private native_confirm_atspi_linux.sh).
    if let Ok(probe) = std::env::var("VW_ATSPI_PROBE") {
        screen_reader_presses(&app, &text, &probe);
    }
    browser_pairing_dialogs(&app);

    // Closing the window declines.
    let mut waiting = Box::pin(show_dialog(app.handle(), text.clone(), &names, true, None));
    assert_eq!(poll_once(&mut waiting), Poll::Pending);
    let dialog = only_window();
    dialog.response(gtk::ResponseType::DeleteEvent);
    pump();
    assert_eq!(poll_once(&mut waiting), Poll::Ready(None));
    assert!(!dialog.is_visible());

    // An agent's request is not modal: the main window stays usable while it waits.
    // It is listed under its ask id while it is open, so its chat can bring it to the front.
    let mut waiting = Box::pin(show_dialog(
        app.handle(),
        text.clone(),
        &names,
        false,
        Some("guard-own-window"),
    ));
    assert_eq!(poll_once(&mut waiting), Poll::Pending);
    pump();
    let dialog = only_window();
    assert!(dialog.is_visible());
    assert!(!dialog.is_modal());
    assert!(super::super::raise_prompt("guard-own-window"));
    pump();
    assert!(dialog.is_visible(), "raising never answers");
    assert_eq!(poll_once(&mut waiting), Poll::Pending);
    dialog.response(gtk::ResponseType::DeleteEvent);
    pump();
    assert_eq!(poll_once(&mut waiting), Poll::Ready(None));
    assert!(
        !super::super::raise_prompt("guard-own-window"),
        "gone with its window"
    );

    // A request nobody waits for any more closes its window; a late click does nothing.
    let mut waiting = Box::pin(show_dialog(app.handle(), text, &names, true, None));
    assert_eq!(poll_once(&mut waiting), Poll::Pending);
    pump();
    let dialog = only_window();
    assert!(dialog.is_visible());
    drop(waiting);
    pump();
    assert!(!dialog.is_visible(), "the dropped request's window is gone");
    assert!(
        !gtk::Window::list_toplevels()
            .iter()
            .any(gtk::prelude::WidgetExt::is_visible)
    );
    dialog.response(gtk::ResponseType::Other(1));
    pump();
    assert!(
        !gtk::Window::list_toplevels()
            .iter()
            .any(gtk::prelude::WidgetExt::is_visible)
    );
}
