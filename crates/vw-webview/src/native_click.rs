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
        "clicking the native preview",
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
    .map_err(|error| match error {
        EngineError::Platform(ref text)
            if matches!(
                text.as_str(),
                "clicking the native preview: native preview application is not active"
                    | "clicking the native preview: native preview window is not key"
            ) =>
        {
            EngineError::FocusRequired
        }
        other => other,
    })
}
