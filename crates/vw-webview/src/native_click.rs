//! Scoped native mouse click for one Tauri child webview. `TauriEngine` calls this after confirming the current
//! mount, visibility, delegated permission and live target. The OS adapter rechecks the current native surface.
//!
//! Depends on: Tauri `with_webview`, vw-platform native click. Called by: `engine`.

use std::sync::Arc;
use std::time::Duration;

use tauri::{Webview, Wry};
use vw_preview::{EngineError, NativeClickGuard, PersonPointer};

/// Sends a native press and release at `point` in a live page viewport measured in CSS pixels.
///
/// # Errors
/// Refuses invalid geometry, an inactive or obscured native surface, or a failed UI-thread operation. Return means
/// the native events were submitted, not that the page handled them or marked their DOM events trusted.
pub(crate) async fn native_click(
    webview: &Webview<Wry>,
    point: (f64, f64),
    viewport: (f64, f64),
    guard: NativeClickGuard,
    person: Arc<PersonPointer>,
) -> Result<(), EngineError> {
    let NativeClickGuard {
        validation,
        sent,
        allowed,
        person: interval,
    } = guard;
    let normalized =
        vw_platform::native_click::normalized(point, viewport).map_err(EngineError::Platform)?;
    crate::platform::with_native(
        webview,
        CLICKING,
        Duration::from_secs(5),
        move |native, reply| {
            // Stop or a timeout dropped the awaiting action before this queued UI callback ran.
            if reply.is_closed() {
                return;
            }
            if let Some(script) = validation {
                match vw_preview::checked_frame_request(
                    &script,
                    cfg!(any(target_os = "linux", target_os = "macos", windows)),
                ) {
                    Ok(request) => {
                        crate::frame_click::click(
                            native,
                            normalized,
                            request,
                            crate::frame_click::Input {
                                sent,
                                allowed,
                                person,
                                interval,
                            },
                            reply,
                        );
                    }
                    Err(reason) => {
                        let _ = reply.send(Err(reason));
                    }
                }
            } else {
                if allowed.as_ref().is_some_and(|allowed| !allowed()) {
                    let _ = reply.send(Err(
                        "Not run: browser permission changed before native input".to_owned(),
                    ));
                    return;
                }
                #[cfg(target_os = "linux")]
                let result =
                    vw_platform::native_click::linux_click(&native.inner(), normalized, &sent);
                #[cfg(windows)]
                let result = vw_platform::native_click::windows_click(
                    &native.controller(),
                    normalized,
                    &sent,
                );
                #[cfg(target_os = "macos")]
                let result =
                    vw_platform::native_click::macos_click(native.inner(), normalized, &sent);
                let _ = reply.send(result);
            }
        },
    )
    .await
    .map_err(focus_required)
}

/// The UI-thread step's name; [`crate::platform::with_native`] puts it in front of the adapter's reason.
const CLICKING: &str = "clicking the native preview";

/// The OS adapters' one not-in-front outcome ([`vw_platform::native_click::NOT_IN_FRONT`], the same on every OS for
/// click, wheel and keyboard input) becomes [`EngineError::FocusRequired`], the agent's plain "bring it forward"
/// answer. Every other platform error stays as it is.
fn focus_required(error: EngineError) -> EngineError {
    let reason = match &error {
        EngineError::Platform(text) => text
            .strip_prefix(CLICKING)
            .and_then(|reason| reason.strip_prefix(": ")),
        _ => None,
    };
    match reason {
        Some(vw_platform::native_click::NOT_IN_FRONT) => EngineError::FocusRequired,
        // An open menu (macOS): the adapter's own plain sentence, without the internal step name.
        Some(vw_platform::native_click::MENU_OPEN) => {
            EngineError::Platform(vw_platform::native_click::MENU_OPEN.to_owned())
        }
        _ => error,
    }
}

#[cfg(test)]
mod tests {
    // PREDICT (before the fix): fails for Windows and Linux. Only the macOS refusals ("application is not active" /
    // "window is not key") were mapped to FocusRequired; the Windows ("host is not visible" / "window is not
    // foreground") and Linux ("is not visible" / "window is not active", and the keyboard's "needs the visible
    // active browser pane") refusals reached the agent as raw internal errors.
    use super::{CLICKING, EngineError, focus_required};
    use vw_platform::native_click::require_front;

    fn adapter(reason: &str) -> EngineError {
        focus_required(EngineError::Platform(format!("{CLICKING}: {reason}")))
    }

    #[test]
    fn every_os_reports_a_window_not_in_front_as_focus_required() {
        // (adapter, condition, shown, front) as each OS adapter reads its live window.
        for (os, condition, shown, front) in [
            (
                "macOS click+wheel+keyboard",
                "app not active / window not key",
                true,
                false,
            ),
            (
                "macOS click+wheel+keyboard",
                "window or view hidden",
                false,
                true,
            ),
            (
                "Windows click+sequence+keyboard",
                "root not foreground",
                true,
                false,
            ),
            (
                "Windows click+sequence+keyboard",
                "host hidden to the tray / minimized",
                false,
                false,
            ),
            (
                "Linux click+wheel+keyboard",
                "toplevel not active",
                true,
                false,
            ),
            (
                "Linux click+wheel+keyboard",
                "widget unmapped / window not viewable",
                false,
                true,
            ),
        ] {
            let reason = require_front(shown, front).expect_err(condition);
            assert!(
                matches!(adapter(&reason), EngineError::FocusRequired),
                "{os}: {condition}: {reason}"
            );
        }
        assert!(require_front(true, true).is_ok());
    }

    // PREDICT (before the fix): fails: no adapter refused for an open menu, so the macOS press waited for the menu and
    // timed out as "the preview did not answer in time (clicking the native preview)". Now the adapter's plain
    // sentence reaches the agent without the internal step name.
    #[test]
    fn an_open_menu_reaches_the_agent_as_its_own_sentence() {
        let EngineError::Platform(text) = adapter(vw_platform::native_click::MENU_OPEN) else {
            panic!("an open menu is not a focus request");
        };
        assert_eq!(text, vw_platform::native_click::MENU_OPEN);
        assert!(
            text.contains("close the menu")
                && text.contains("nothing was sent")
                && !text.contains("with select"),
            "{text}"
        );
    }

    #[test]
    fn other_platform_refusals_keep_their_own_text() {
        for reason in [
            "native preview point is occluded",
            "native preview point is not over its browser child",
            "native preview has invalid click geometry",
            "Not run: the page target changed before native input",
        ] {
            assert!(
                matches!(adapter(reason), EngineError::Platform(ref text) if text.ends_with(reason)),
                "{reason}"
            );
        }
        // The outcome only counts as the adapter's own answer to this step, never inside other text.
        let wrapped = format!(
            "Native input may have been sent: {}",
            vw_platform::native_click::NOT_IN_FRONT
        );
        assert!(matches!(adapter(&wrapped), EngineError::Platform(_)));
    }
}
