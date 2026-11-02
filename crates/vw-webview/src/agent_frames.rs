//! Routes native frame scripts behind the preview engine seam. Uses platform handles, never page IPC.
//! The builder installs registries before navigation; close releases them on the UI thread.
use tauri::{Webview, Wry};
use vw_preview::{EngineError, FrameAnswer};

#[cfg_attr(target_os = "linux", allow(clippy::unused_async))]
pub(crate) async fn install(webview: &Webview<Wry>) -> Result<(), EngineError> {
    #[cfg(target_os = "macos")]
    {
        crate::mac_agent::install_world(webview).await?;
    }
    #[cfg(windows)]
    {
        crate::platform::with_native(
            webview,
            "installing browser frames",
            std::time::Duration::from_secs(5),
            |native, reply| {
                crate::frames_win::install(
                    &native,
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
            },
        )
        .await?;
    }
    #[cfg(target_os = "linux")]
    let _ = webview;
    Ok(())
}

pub(crate) async fn evaluate(
    webview: &Webview<Wry>,
    frame: Option<String>,
    script: String,
) -> Result<Vec<FrameAnswer>, EngineError> {
    #[cfg(target_os = "linux")]
    {
        crate::linux_agent::evaluate(webview, frame, script).await
    }
    #[cfg(windows)]
    {
        crate::platform::with_native(
            webview,
            "evaluating browser frames",
            std::time::Duration::from_secs(5),
            move |native, reply| {
                crate::frames_win::evaluate(
                    &native,
                    frame.as_deref(),
                    &script,
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
            },
        )
        .await
    }
    #[cfg(target_os = "macos")]
    {
        crate::platform::with_native(webview, "evaluating browser frames", std::time::Duration::from_secs(5), move |native, reply| {
        let targeted = frame.is_some();
        let done: vw_platform::webview::agent_world_mac::FramesDone = Box::new(move |result| {
            let _ = reply.send(result.and_then(|answers| answers.into_iter().filter(|a| targeted || a.value.is_ok()).map(|a| {
                a.value.map(|value| FrameAnswer { id:a.id, generation:a.generation, main:a.main, url:a.url, origin: None, value })
                    .map_err(|_| "frame document changed, detached, or could not be evaluated".to_owned())
            }).collect()));
        });
        if let Some(frame) = frame { vw_platform::webview::agent_world_mac::evaluate_registered(native.inner(), &frame, &script, done); }
        else { vw_platform::webview::agent_world_mac::evaluate_in_frames(native.inner(), &script, done); }
    }).await
    }
}

/// An act's evaluation that counts as the person's gesture: Linux through the UI process, Windows with the
/// protocol's `userGesture` (and picker interception), macOS the ordinary route (its evaluation already does).
pub(crate) async fn evaluate_as_input(
    webview: &Webview<Wry>,
    frame: Option<String>,
    script: String,
) -> Result<Vec<FrameAnswer>, EngineError> {
    #[cfg(target_os = "linux")]
    {
        crate::linux_agent::evaluate_as_input(webview, frame, script).await
    }
    #[cfg(windows)]
    {
        crate::platform::with_native(
            webview,
            "evaluating browser frames",
            std::time::Duration::from_secs(5),
            move |native, reply| {
                crate::frames_win::evaluate_input(
                    &native,
                    frame.as_deref(),
                    &script,
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
            },
        )
        .await
    }
    #[cfg(target_os = "macos")]
    {
        evaluate(webview, frame, script).await
    }
}

pub(crate) fn forget(webview: &Webview<Wry>) -> Result<(), EngineError> {
    webview
        .with_webview(|native| {
            #[cfg(windows)]
            crate::frames_win::forget(&native);
            #[cfg(target_os = "macos")]
            {
                vw_platform::webview::agent_world_mac::forget_agent_world(native.inner());
                vw_platform::webview::full_screen_mac::forget(native.inner());
            }
            #[cfg(target_os = "linux")]
            let _ = native;
        })
        .map_err(|e| EngineError::Platform(e.to_string()))
}
