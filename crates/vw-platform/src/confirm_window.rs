//! The OS confirmation window of macOS and Windows: a question, the text below it and up to three buttons, answered
//! only by a press of one of its buttons, closed from outside when the request that asks ends without an answer.
//! (Linux uses GTK's `MessageDialog` directly in the app.)
//!
//! - macOS ([`show`] / [`close`], main thread): an `NSAlert` shown as a sheet on the given window (or as a window of
//!   its own), one sheet at a time per process in the order they were asked; later ones wait and are shown when it
//!   ends. The first button has Return (the blue default button), Escape declines, the other buttons have no key. A
//!   press with the pointer, Space with Full Keyboard Access, `VoiceOver`'s VO+Space and Voice Control's "Click …" are
//!   all the button's own action. The question is the bold message text and the window's accessibility title.
//! - Windows ([`show`] / [`OpenConfirmation`], any thread): a common-controls v6 task dialog on a thread of its own,
//!   modal over the given window; the first button is the default and starts with the focus (Return, Space), Escape
//!   and the title bar's close button decline, Tab and the arrow keys move between the buttons, UI Automation's
//!   Invoke (Narrator, `NVDA`, Voice Access) presses a button. The window title is [`ConfirmSpec::title`], the question
//!   the main instruction.
//!
//! A window of its own (no parent) can go behind the main window; `raise` brings it back to the front (macOS: the one
//! shown; Windows: [`OpenConfirmation::raiser`]). It never answers.
//!
//! The answer is the index of the pressed button in [`ConfirmSpec::buttons`], or `None`: declined without a button
//! (Escape, the close button), closed by [`close`] / dropping the handle, or a window that failed to show. A press
//! after the window was closed from outside is never reported.
//!
//! Depends on: `AppKit` through objc2 (macOS), comctl32 v6 through the `windows` crate (Windows; the app's manifest asks
//! for common controls v6). Called by: the app (`app/src/dialogs/*_confirmation.rs`).

#[cfg(target_os = "macos")]
#[path = "confirm_window/mac.rs"]
mod imp;
#[cfg(windows)]
#[path = "confirm_window/win.rs"]
mod imp;

#[cfg(windows)]
pub use imp::{OpenConfirmation, Raise, show};
#[cfg(target_os = "macos")]
pub use imp::{close, raise, show};

/// What one confirmation window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmSpec {
    /// The window title (Windows title bar). macOS shows no title on an alert; it uses [`ConfirmSpec::heading`]
    /// as the window's accessibility title instead.
    pub title: String,
    /// The question: bold first line, read first by screen readers.
    pub heading: String,
    /// The text below the question (may be empty).
    pub body: String,
    /// The buttons in the order they are shown, 1 to 3. The FIRST is the default button (Return): the declining one.
    pub buttons: Vec<String>,
}

/// How a confirmation answers: the index of the pressed button in [`ConfirmSpec::buttons`], or `None`.
pub type Reply = Box<dyn FnOnce(Option<usize>) + Send>;
