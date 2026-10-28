//! Linux confirmation window with cancellation tied to the awaiting request.
//!
//! Depends on: GTK on Tauri's main thread, a Tokio one-shot reply, vw-host (heading, button order). Called by:
//! `dialogs::show_dialog`.
//! A dropped request destroys its own window; late answers cannot approve a canceled action. The last button (the
//! declining one) is the default and has the focus; the question is the window's primary text.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use gtk::prelude::*;
use tauri::{AppHandle, Runtime};
use tokio::sync::oneshot;

thread_local! {
    static WINDOWS: RefCell<HashMap<u64, gtk::MessageDialog>> = RefCell::new(HashMap::new());
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct WindowOwner<R: Runtime> {
    app: AppHandle<R>,
    id: u64,
    canceled: Arc<AtomicBool>,
}

impl<R: Runtime> Drop for WindowOwner<R> {
    fn drop(&mut self) {
        self.canceled.store(true, Ordering::Release);
        let id = self.id;
        let _ = self.app.run_on_main_thread(move || {
            if let Some(dialog) = WINDOWS.with(|windows| windows.borrow_mut().remove(&id)) {
                // A GTK dialog may handle close as a response without destroying itself.
                // Hide first so a canceled request cannot leave an actionable window behind.
                dialog.hide();
                dialog.close();
            }
        });
    }
}

/// The label a dialog response stands for: button `n` (`Other(n)`, counted from 1) is `labels[n - 1]`.
/// Closing the window, any other response and every answer to a canceled request are `None`
/// (declined), so a late click cannot approve an action nobody waits for any more.
fn answer_for(response: gtk::ResponseType, labels: &[String], canceled: bool) -> Option<String> {
    if canceled {
        return None;
    }
    match response {
        gtk::ResponseType::Other(index) if index > 0 => labels.get(usize::from(index - 1)).cloned(),
        _ => None,
    }
}

/// Shows one confirmation and returns a pressed button. Dropping this future closes its window.
pub(super) async fn show_dialog<R: Runtime>(
    app: &AppHandle<R>,
    text: String,
    labels: &[String],
    modal: bool,
    ask: Option<&str>,
) -> Option<String> {
    if !(1..=3).contains(&labels.len()) {
        return None;
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let canceled = Arc::new(AtomicBool::new(false));
    let owner = WindowOwner {
        app: app.clone(),
        id,
        canceled: canceled.clone(),
    };
    let (send, receive) = oneshot::channel();
    let labels = labels.to_vec();
    let app_for_window = app.clone();
    if app
        .run_on_main_thread(move || {
            // Cancellation can race with posting this closure. A canceled request must never show a dialog.
            if canceled.load(Ordering::Acquire) {
                return;
            }
            let parent =
                crate::main_window(&app_for_window).and_then(|window| window.gtk_window().ok());
            // The question is the bold first line (Orca reads it right after the window title), the rest below it.
            let (heading, rest) = vw_host::dialog_heading(&text);
            let dialog = gtk::MessageDialog::new(
                parent.as_ref(),
                // Kept above the main window; modal over it except for an agent's request.
                if modal {
                    gtk::DialogFlags::MODAL
                } else {
                    gtk::DialogFlags::empty()
                },
                gtk::MessageType::Warning,
                gtk::ButtonsType::None,
                &heading,
            );
            dialog.set_secondary_text((!rest.is_empty()).then_some(rest.as_str()));
            dialog.set_title(vw_host::DIALOG_TITLE);
            // GTK names a warning dialog "Warning" for screen readers; Orca should say who asks instead.
            if let Some(accessible) = dialog.accessible() {
                gtk::atk::prelude::AtkObjectExt::set_name(&accessible, vw_host::DIALOG_TITLE);
            }
            for (index, label) in labels.iter().enumerate() {
                let Ok(response) = u16::try_from(index + 1) else {
                    return;
                };
                dialog.add_button(label, gtk::ResponseType::Other(response));
            }
            // The last button declines (`vw_host::dialog_buttons`): it is the default and holds the focus, so Return
            // never approves. GtkDialog moves the focus to the default button when it is shown.
            if let Ok(last) = u16::try_from(labels.len()) {
                dialog.set_default_response(gtk::ResponseType::Other(last));
            }
            let send = RefCell::new(Some(send));
            let canceled_on_response = canceled.clone();
            dialog.connect_response(move |_, response| {
                let answer = answer_for(
                    response,
                    &labels,
                    canceled_on_response.load(Ordering::Acquire),
                );
                let send = send.borrow_mut().take();
                if let Some(dialog) = WINDOWS.with(|windows| windows.borrow_mut().remove(&id)) {
                    dialog.hide();
                    dialog.close();
                }
                if let Some(send) = send {
                    let _ = send.send(answer);
                }
            });
            WINDOWS.with(|windows| windows.borrow_mut().insert(id, dialog.clone()));
            dialog.show_all();
        })
        .is_err()
    {
        return None;
    }
    let handle = app.clone();
    let _listed = super::listed(
        ask,
        Arc::new(move || {
            handle
                .run_on_main_thread(move || {
                    if let Some(dialog) = WINDOWS.with(|windows| windows.borrow().get(&id).cloned())
                    {
                        dialog.present();
                    }
                })
                .is_ok()
        }),
    );
    let answer = receive.await.ok().flatten();
    drop(owner);
    answer
}

#[cfg(test)]
mod tests {
    //! The answer mapping runs everywhere. The window tests need an X display and Tauri's mock
    //! runtime (its main-thread tasks run at once on the calling thread, so the GTK window lives on
    //! the test's thread): `xvfb-run -a cargo test -p app --lib -- --ignored linux_confirmation`.
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll, Waker};

    use super::*;

    fn labels(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_button_answers_its_label_and_everything_else_declines() {
        let three = labels(&["Allow once", "Always", "Deny"]);
        let answer = |response| answer_for(response, &three, false);
        assert_eq!(
            answer(gtk::ResponseType::Other(1)).as_deref(),
            Some("Allow once")
        );
        assert_eq!(
            answer(gtk::ResponseType::Other(2)).as_deref(),
            Some("Always")
        );
        assert_eq!(answer(gtk::ResponseType::Other(3)).as_deref(), Some("Deny"));
        for declined in [
            gtk::ResponseType::Other(0),
            gtk::ResponseType::Other(4),
            gtk::ResponseType::DeleteEvent,
            gtk::ResponseType::Close,
            gtk::ResponseType::Cancel,
            gtk::ResponseType::Ok,
            gtk::ResponseType::Accept,
            gtk::ResponseType::None,
        ] {
            assert_eq!(answer(declined), None, "{declined:?}");
        }
        // A click that arrives after the request was given up never approves.
        assert_eq!(answer_for(gtk::ResponseType::Other(1), &three, true), None);
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
        assert!(WINDOWS.with(|windows| windows.borrow().is_empty()));
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
        WINDOWS.with(|windows| {
            let windows = windows.borrow();
            assert_eq!(windows.len(), 1, "exactly one confirmation window");
            windows.values().next().cloned().unwrap()
        })
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

    /// Orca's way: the probe finds the window in the accessibility tree, prints what it reads (role, text in order,
    /// buttons with their states) and presses one button; the press answers that button.
    fn screen_reader_presses(app: &tauri::App<tauri::test::MockRuntime>, text: &str, probe: &str) {
        let three = labels(&["Allow once", "Always", "Deny"]);
        for label in ["Deny", "Allow once"] {
            let asked = format!("{text}\n\n{}", vw_host::AGENT_LINE);
            let mut waiting = Box::pin(show_dialog(app.handle(), asked, &three, true, None));
            assert_eq!(poll_once(&mut waiting), Poll::Pending);
            pump_for(300);
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
            assert!(said.contains("BUTTON Deny | focused default"), "{said}");
            pump_for(600);
            assert_eq!(
                poll_once(&mut waiting),
                Poll::Ready(Some(label.to_owned())),
                "{label}"
            );
        }
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
        assert!(WINDOWS.with(|windows| windows.borrow().is_empty()));

        the_declining_button_is_the_default(&app, &text);
        real_keys_reach_the_buttons(&app, &text);
        // A screen reader's press through the accessibility tree: only where a script provides the probe and an
        // accessibility bus (the private native_confirm_atspi_linux.sh).
        if let Ok(probe) = std::env::var("VW_ATSPI_PROBE") {
            screen_reader_presses(&app, &text, &probe);
        }

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
        assert!(WINDOWS.with(|windows| windows.borrow().is_empty()));
        dialog.response(gtk::ResponseType::Other(1));
        pump();
        assert!(WINDOWS.with(|windows| windows.borrow().is_empty()));
    }
}
