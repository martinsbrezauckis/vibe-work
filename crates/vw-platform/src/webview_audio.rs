//! Native output controls used by the preview engine while a voice answer listens.
//! Runs only inside Tauri's UI-thread native-webview callback; page code cannot undo these controls.

#[cfg(windows)]
/// Set native output mute, returning the previous setting.
///
/// # Errors
/// The runtime lacks the mute interface or refuses the operation.
pub fn mute(
    controller: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller,
    muted: bool,
) -> Result<bool, String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
    use windows::core::Interface as _;
    // SAFETY: the caller holds Tauri's live controller on its UI thread; COM out parameters are initialized.
    unsafe {
        let view: ICoreWebView2_8 = controller
            .CoreWebView2()
            .and_then(|v| v.cast())
            .map_err(|e| e.to_string())?;
        let mut previous = windows::core::BOOL::default();
        view.IsMuted(&raw mut previous).map_err(|e| e.to_string())?;
        view.SetIsMuted(muted).map_err(|e| e.to_string())?;
        Ok(previous.as_bool())
    }
}

#[cfg(target_os = "macos")]
/// Suspend every media source until the paired resume, acknowledging completion before microphone capture.
pub fn suspend(
    view: *mut std::ffi::c_void,
    suspended: bool,
    done: Box<dyn FnOnce(Result<bool, String>) + Send>,
) {
    use block2::RcBlock;
    use objc2::{msg_send, sel};
    use objc2_web_kit::WKWebView;
    if view.is_null() {
        done(Err("Browser is unavailable".into()));
        return;
    }
    // SAFETY: Tauri provides a live object on the UI thread. Query before invoking a newer WebKit API.
    let available: bool = unsafe {
        msg_send![&*view.cast::<WKWebView>(), respondsToSelector: sel!(setAllMediaPlaybackSuspended:completionHandler:)]
    };
    if !available {
        done(Err(
            "Browser audio cannot be suspended on this system".into()
        ));
        return;
    }
    let done = std::sync::Mutex::new(Some(done));
    let completion = RcBlock::new(move || {
        if let Some(done) = done
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            done(Ok(false));
        }
    });
    // SAFETY: Tauri provides a live WKWebView on the UI thread. WebKit retains the completion block.
    unsafe {
        (&*view.cast::<WKWebView>())
            .setAllMediaPlaybackSuspended_completionHandler(suspended, Some(&completion));
    }
}
