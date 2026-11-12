//! Linux GTK confirmation windows, owned on the main thread and closed when their request ends.
//! Uses GTK, the shared confirmation specification and a one-shot callback; called by app dialog wiring.
use super::ConfirmSpec;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    static WINDOWS: RefCell<HashMap<u64, gtk::MessageDialog>> = RefCell::new(HashMap::new());
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

/// Shows one Linux confirmation on the GTK main thread. The last button is the declining default.
/// The caller supplies a unique live id and keeps the cancellation flag until its reply is received.
pub fn show(
    id: u64,
    parent: Option<&gtk::ApplicationWindow>,
    spec: &ConfirmSpec,
    modal: bool,
    canceled: &Arc<AtomicBool>,
    reply: Box<dyn FnOnce(Option<String>) + Send>,
) {
    let labels = spec.buttons.clone();
    let dialog = gtk::MessageDialog::new(
        parent,
        // Kept above the main window; modal over it except for an agent's request.
        if modal {
            gtk::DialogFlags::MODAL
        } else {
            gtk::DialogFlags::empty()
        },
        gtk::MessageType::Warning,
        gtk::ButtonsType::None,
        &spec.heading,
    );
    dialog.set_secondary_text((!spec.body.is_empty()).then_some(spec.body.as_str()));
    dialog.set_title(&spec.title);
    // GTK names a warning dialog "Warning" for screen readers; Orca should say who asks instead.
    if let Some(accessible) = dialog.accessible() {
        gtk::atk::prelude::AtkObjectExt::set_name(&accessible, &spec.title);
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
    let send = RefCell::new(Some(reply));
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
            send(answer);
        }
    });
    WINDOWS.with(|windows| windows.borrow_mut().insert(id, dialog.clone()));
    dialog.show_all();
    if modal {
        dialog.present();
    }
}

/// Hides and closes this request's window on the GTK main thread.
pub fn close(id: u64) {
    if let Some(dialog) = WINDOWS.with(|windows| windows.borrow_mut().remove(&id)) {
        // Hide first: GTK can report close as a response without destroying the dialog.
        dialog.hide();
        dialog.close();
    }
}

/// Presents an existing request without answering it; must run on the GTK main thread.
pub fn raise(id: u64) {
    if let Some(dialog) = WINDOWS.with(|windows| windows.borrow().get(&id).cloned()) {
        dialog.present();
    }
}

#[cfg(test)]
mod tests {
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
}
