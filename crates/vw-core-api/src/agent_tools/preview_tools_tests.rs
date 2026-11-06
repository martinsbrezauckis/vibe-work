//! Preview tool reply tests using retained view/visibility states; called by core unit tests.

use super::open_note;
use vw_preview::{DeviceWidth, PreviewServerState, PreviewState, PreviewView};

fn view() -> PreviewView {
    PreviewView {
        session_id: "s".into(),
        url: Some("http://127.0.0.1:28100/".into()),
        title: None,
        state: PreviewState::Hidden,
        server: None,
        loading: false,
        waiting_for_browser: false,
        width: DeviceWidth::Fit,
        downloads: Vec::new(),
        refused: Vec::new(),
        console_count: 0,
    }
}

#[test]
fn window_visibility_precedes_a_retained_stopped_server_marker() {
    let mut stopped = view();
    stopped.state = PreviewState::Stopped;
    stopped.server = Some(PreviewServerState::Stopped);
    let reason = "The Vibe Work window is minimized or hidden, so App preview cannot be captured. Ask the user to show the window, or check App preview errors with preview_console.";
    assert_eq!(
        open_note(&stopped, Some(reason.to_owned())),
        format!("screenshot unavailable: {reason}")
    );
}

#[test]
fn open_answer_names_away_reasons_without_width_advice() {
    for reason in [
        "The Vibe Work window is minimized or hidden, so App preview cannot be captured. Ask the user to show it, or check App preview errors with preview_console.",
        "The user is viewing another session, so App preview cannot be captured. Ask the user to open this session, or check App preview errors with preview_console.",
    ] {
        let answer = open_note(&view(), Some(reason.to_owned()));
        assert_eq!(
            answer,
            format!("loaded, but screenshot unavailable: {reason}")
        );
        assert!(!answer.contains("preview_open with a width"));
    }
    let mut stopped = view();
    stopped.state = PreviewState::Stopped;
    stopped.server = Some(PreviewServerState::Stopped);
    assert_eq!(
        open_note(&stopped, None),
        "The App preview remains stopped. Start its server if needed, then Reload."
    );
}
