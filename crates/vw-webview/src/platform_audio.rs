//! Native playback silence and restoration on the UI thread; called by the engine's scoped audio lease.
use crate::platform::with_native;
use std::time::Duration;
use tauri::{Webview, Wry};
use vw_preview::EngineError;

/// Native mute/suspension. The prior value is recorded on the UI thread, even if the awaiting task times out.
pub(crate) async fn quiet_audio(
    webview: &Webview<Wry>,
    saved: std::sync::Arc<std::sync::Mutex<Option<bool>>>,
    quiet: bool,
) -> Result<(), EngineError> {
    with_native(
        webview,
        "browser audio",
        Duration::from_secs(5),
        move |native, reply| {
            let target = if quiet {
                true
            } else {
                let Some(previous) = *saved
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                else {
                    let _ = reply.send(Ok(()));
                    return;
                };
                previous
            };
            #[cfg(target_os = "linux")]
            {
                use webkit2gtk::WebViewExt as _;
                let view = native.inner();
                if quiet {
                    *saved
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(view.is_muted());
                }
                view.set_is_muted(target);
                let _ = reply.send(Ok(()));
            }
            #[cfg(windows)]
            {
                let result = vw_platform::webview_audio::mute(&native.controller(), target).map(
                    |previous| {
                        if quiet {
                            *saved
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                Some(previous);
                        }
                    },
                );
                let _ = reply.send(result);
            }
            #[cfg(target_os = "macos")]
            {
                if quiet {
                    *saved
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(false);
                }
                vw_platform::webview_audio::suspend(
                    native.inner().cast(),
                    target,
                    Box::new(move |result| {
                        let _ = reply.send(result.map(|_| ()));
                    }),
                );
            }
        },
    )
    .await
}

/// Acknowledge previously queued native closes before reporting that all browser output is quiet.
pub(crate) async fn barrier(webview: &Webview<Wry>) -> Result<(), EngineError> {
    with_native(
        webview,
        "browser audio barrier",
        Duration::from_secs(5),
        |_, reply| {
            let _ = reply.send(Ok(()));
        },
    )
    .await
}

/// Native single-tab mute and readback; macOS exposes suspension, which is not tab mute.
pub(crate) async fn tab_audio(
    webview: &Webview<Wry>,
    muted: Option<bool>,
) -> Result<serde_json::Value, EngineError> {
    with_native(webview, "tab audio", Duration::from_secs(5), move |native, reply| {
        #[cfg(target_os = "linux")]
        {
            use webkit2gtk::WebViewExt as _;
            let view = native.inner();
            if let Some(muted) = muted { view.set_is_muted(muted); }
            let _ = reply.send(Ok(serde_json::json!({"muted":view.is_muted(),"audible":view.is_playing_audio()})));
        }
        #[cfg(windows)]
        { let _ = reply.send(vw_platform::webview_audio::state(&native.controller(), muted).map(|(muted,audible)| serde_json::json!({"muted":muted,"audible":audible}))); }
        #[cfg(target_os = "macos")]
        { let _ = (native, muted); let _ = reply.send(Err("This engine exposes playback suspension, not native tab mute or audible state; use media.read and media.mute for individual elements.".into())); }
    }).await
}
