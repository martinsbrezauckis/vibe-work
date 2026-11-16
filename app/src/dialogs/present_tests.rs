//! Which confirmations bring the main window forward.
//!
//! PREDICT: before the change the macOS window called `reveal_main_window` for every confirmation, an agent's own
//! window included, so `no_os_window_reveals_the_main_window_by_itself` failed on `macos_confirmation.rs` (and on
//! the Windows file, which revealed through its own `if modal`), and `present_for` did not exist.
//! After: the per-OS windows ask [`super::present_for`], which reveals only for a modal confirmation; an agent's
//! request (a window of its own) leaves the main window where it is, and the waiting chat's badge plus the app
//! badge say that it waits.

use serde_json::json;
use vw_gate::{ConfirmationRequest, Requester};

fn request(requester: Requester) -> ConfirmationRequest {
    ConfirmationRequest {
        method: "git.push".to_owned(),
        summary: "Push to origin".to_owned(),
        params: json!({ "askId": "ask-1" }),
        requester,
        always: None,
    }
}

#[test]
fn only_a_modal_confirmation_brings_the_main_window_forward() {
    assert!(!super::reveals_main_window(super::modal(&request(
        Requester::Agent
    ))));
    for requester in [Requester::Window, Requester::DebugApi] {
        assert!(super::reveals_main_window(super::modal(&request(
            requester
        ))));
    }
}

/// The OS windows never call the reveal helper themselves: the one decision is `present_for`.
#[test]
fn no_os_window_reveals_the_main_window_by_itself() {
    for (name, source) in [
        (
            "macos_confirmation.rs",
            include_str!("macos_confirmation.rs"),
        ),
        (
            "windows_confirmation.rs",
            include_str!("windows_confirmation.rs"),
        ),
        (
            "linux_confirmation.rs",
            include_str!("linux_confirmation.rs"),
        ),
    ] {
        assert!(
            !source.contains("reveal_main_window("),
            "{name} brings the main window forward without asking present_for"
        );
    }
}
