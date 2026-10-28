//! Windows confirmation window (see `confirm_window.rs`): a common-controls v6 task dialog on a thread of its own,
//! closed from outside by the handle.
//!
//! The handle owns the window: dropping it closes the dialog, so a request that ends without an answer (its deadline)
//! takes its window with it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use windows::Win32::Foundation::{HWND, LPARAM, S_OK, WPARAM};
use windows::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOG_NOTIFICATIONS, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0,
    TD_WARNING_ICON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW,
    TDF_SIZE_TO_CONTENT, TDM_CLICK_BUTTON, TDN_CREATED, TDN_DESTROYED, TaskDialogIndirect,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, IDCANCEL, PostMessageW, SetForegroundWindow,
};
use windows::core::{HRESULT, PCWSTR};

use super::{ConfirmSpec, Reply};

/// The id of the first button; button `n` (in the shown order) is `FIRST_ID + n`. Above the system ids (IDOK 1 ..
/// IDCONTINUE 11), so IDCANCEL (Escape, the close button) is never taken for a button.
const FIRST_ID: i32 = 100;

/// What the dialog's thread and the handle share.
struct Shared {
    /// The dialog's window while it exists (`TDN_CREATED` .. `TDN_DESTROYED`), as a raw handle value.
    window: Mutex<Option<isize>>,
    /// Set by [`OpenConfirmation::close`]: the window closes, and a press that still arrives is not reported.
    closed: AtomicBool,
}

impl Shared {
    /// Tells the dialog to end as cancelled (what Escape does). Any thread; a window that is gone is skipped.
    fn cancel_window(&self) {
        let window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(raw) = *window {
            // SAFETY: the handle is the live task dialog: it is cleared under this lock at TDN_DESTROYED, so it can
            // not be a later window that reused the value. Posting a message is allowed from any thread.
            let _ = unsafe {
                PostMessageW(
                    Some(HWND(raw as *mut _)),
                    u32::try_from(TDM_CLICK_BUTTON.0).unwrap_or_default(),
                    WPARAM(usize::try_from(IDCANCEL.0).unwrap_or_default()),
                    LPARAM(0),
                )
            };
        }
    }
}

/// The task dialog's notifications: keeps its window while it exists, and cancels it at once when it was closed from
/// outside before it was created.
unsafe extern "system" fn notified(
    hwnd: HWND,
    msg: TASKDIALOG_NOTIFICATIONS,
    _wparam: WPARAM,
    _lparam: LPARAM,
    data: isize,
) -> HRESULT {
    // SAFETY: `data` is the `Shared` that `run` keeps alive (an `Arc` clone) for the whole `TaskDialogIndirect` call.
    let shared = unsafe { &*(data as *const Shared) };
    if msg == TDN_CREATED {
        *shared.window.lock().unwrap_or_else(PoisonError::into_inner) = Some(hwnd.0 as isize);
        if shared.closed.load(Ordering::Acquire) {
            shared.cancel_window();
        }
    } else if msg == TDN_DESTROYED {
        *shared.window.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
    S_OK
}

/// A NUL-terminated UTF-16 copy of `text`.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The answer a dialog result stands for: `FIRST_ID + n` is button `n`; IDCANCEL (Escape, the close button), any
/// other id, a failed dialog and every result after a close from outside are `None`.
fn answer_for(pressed: Option<i32>, buttons: usize, closed: bool) -> Option<usize> {
    if closed {
        return None;
    }
    let index = usize::try_from(pressed?.checked_sub(FIRST_ID)?).ok()?;
    (index < buttons).then_some(index)
}

/// Runs one task dialog on the calling thread until it ends; the pressed id, or `None` when it failed to show.
fn run(parent: isize, spec: &ConfirmSpec, shared: &Arc<Shared>) -> Option<i32> {
    let title = wide(&spec.title);
    let heading = wide(&spec.heading);
    let body = (!spec.body.is_empty()).then(|| wide(&spec.body));
    let labels: Vec<Vec<u16>> = spec.buttons.iter().map(|label| wide(label)).collect();
    let buttons: Vec<TASKDIALOG_BUTTON> = labels
        .iter()
        .zip(FIRST_ID..)
        .map(|(label, id)| TASKDIALOG_BUTTON {
            nButtonID: id,
            pszButtonText: PCWSTR(label.as_ptr()),
        })
        .collect();
    let config = TASKDIALOGCONFIG {
        cbSize: u32::try_from(std::mem::size_of::<TASKDIALOGCONFIG>()).unwrap_or_default(),
        hwndParent: HWND(parent as *mut _),
        // Escape and the close button end it as IDCANCEL; centred on the parent; as wide as the text needs.
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION
            | TDF_POSITION_RELATIVE_TO_WINDOW
            | TDF_SIZE_TO_CONTENT,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        Anonymous1: TASKDIALOGCONFIG_0 {
            pszMainIcon: TD_WARNING_ICON,
        },
        pszMainInstruction: PCWSTR(heading.as_ptr()),
        pszContent: body.as_ref().map_or(PCWSTR::null(), |b| PCWSTR(b.as_ptr())),
        cButtons: u32::try_from(buttons.len()).unwrap_or_default(),
        pButtons: buttons.as_ptr(),
        // The first button (the declining one) is the default and starts with the focus.
        nDefaultButton: FIRST_ID,
        pfCallback: Some(notified),
        lpCallbackData: Arc::as_ptr(shared) as isize,
        ..Default::default()
    };
    let mut pressed = 0i32;
    // SAFETY: every pointer in `config` points into locals of this function (strings, buttons) or into `shared`,
    // all alive until the call returns; `pressed` is a local.
    let shown =
        unsafe { TaskDialogIndirect(&raw const config, Some(&raw mut pressed), None, None) };
    shown.ok().map(|()| pressed)
}

/// An open confirmation window. Dropping it (or [`OpenConfirmation::close`]) closes the window; its reply then gets
/// `None`, and a press that arrives after that is not reported.
pub struct OpenConfirmation(Arc<Shared>);

impl OpenConfirmation {
    /// Closes the window (any thread); nothing happens when it already ended.
    pub fn close(&self) {
        self.0.closed.store(true, Ordering::Release);
        self.0.cancel_window();
    }

    /// Something that brings this window to the front later, from any thread (see [`Raise::raise`]).
    #[must_use]
    pub fn raiser(&self) -> Raise {
        Raise(self.0.clone())
    }
}

/// Brings one confirmation window to the front; it never presses a button.
#[derive(Clone)]
pub struct Raise(Arc<Shared>);

impl Raise {
    /// Brings the window in front of the app's other windows and gives it the focus (any thread). Windows allows
    /// that while this process has the foreground, which it has when the person acts in the main window. False when
    /// the window is not (or no longer) there.
    #[must_use]
    pub fn raise(&self) -> bool {
        let window = self.0.window.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(raw) = *window else {
            return false;
        };
        let hwnd = HWND(raw as *mut _);
        // SAFETY: the handle is the live task dialog (cleared under this lock at TDN_DESTROYED); both calls only
        // change the z-order and the focus.
        unsafe {
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
        }
        true
    }
}

impl Drop for OpenConfirmation {
    fn drop(&mut self) {
        self.close();
    }
}

/// Shows `spec` modal over `parent` (a window handle of this process, or 0 for none) on a new thread and answers
/// `reply` once, from that thread, when it ends.
///
/// # Errors
/// Not 1 to 3 buttons, or the thread could not start (nothing is shown; `reply` is not called).
pub fn show(parent: isize, spec: &ConfirmSpec, reply: Reply) -> Result<OpenConfirmation, String> {
    if !(1..=3).contains(&spec.buttons.len()) {
        return Err("a confirmation window has 1 to 3 buttons".to_owned());
    }
    let shared = Arc::new(Shared {
        window: Mutex::new(None),
        closed: AtomicBool::new(false),
    });
    let mine = shared.clone();
    let spec = spec.clone();
    std::thread::Builder::new()
        .name("vw-confirmation".to_owned())
        .spawn(move || {
            let pressed = run(parent, &spec, &mine);
            reply(answer_for(
                pressed,
                spec.buttons.len(),
                mine.closed.load(Ordering::Acquire),
            ));
        })
        .map_err(|error| format!("the confirmation window's thread did not start: {error}"))?;
    Ok(OpenConfirmation(shared))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_button_answers_its_index_and_everything_else_declines() {
        assert_eq!(answer_for(Some(100), 3, false), Some(0));
        assert_eq!(answer_for(Some(101), 3, false), Some(1));
        assert_eq!(answer_for(Some(102), 3, false), Some(2));
        // IDCANCEL (Escape, the close button), other system ids, ids past the buttons, a failed dialog.
        for declined in [
            Some(IDCANCEL.0),
            Some(1),
            Some(7),
            Some(99),
            Some(103),
            Some(-1),
            None,
        ] {
            assert_eq!(answer_for(declined, 3, false), None, "{declined:?}");
        }
        assert_eq!(
            answer_for(Some(102), 2, false),
            None,
            "past the two buttons"
        );
        // A press that arrives after a close from outside is never reported.
        assert_eq!(answer_for(Some(101), 3, true), None);
    }
}
