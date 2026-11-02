//! Windows: tells the caller each time a `WebView2` page enters or leaves full screen (`ContainsFullScreenElement`),
//! so an agent act's full screen can be left again at once (`WebView2` has no way to refuse it before it starts).
//!
//! Depends on: `webview2-com`. Called by: vw-webview's act watch at mount, on the webview's UI thread.

use webview2_com::{
    ContainsFullScreenElementChangedEventHandler,
    Microsoft::Web::WebView2::Win32::ICoreWebView2Controller,
};
use windows::core::BOOL;

/// Reads the view's current full-screen state after an exit command completes.
///
/// # Errors
/// The view has closed or its state could not be read. Call on the controller's UI thread.
pub fn contains(controller: &ICoreWebView2Controller) -> Result<bool, String> {
    let mut entered = BOOL::default();
    // SAFETY: the caller holds this controller on its UI thread; this only reads its state.
    unsafe {
        controller
            .CoreWebView2()
            .and_then(|core| core.ContainsFullScreenElement(&raw mut entered))
    }
    .map_err(|e| format!("reading browser full screen: {e}"))?;
    Ok(entered.as_bool())
}

/// `on_change(entered)` runs on every change of the page's full-screen state, until the webview closes.
///
/// # Errors
/// A closed webview, or the event could not be registered.
pub fn on_change(
    controller: &ICoreWebView2Controller,
    on_change: Box<dyn Fn(bool)>,
) -> Result<(), String> {
    // SAFETY: Tauri hands over the live controller on its UI thread; this only reads it.
    let core =
        unsafe { controller.CoreWebView2() }.map_err(|e| format!("WebView2 full screen: {e}"))?;
    let handler =
        ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _| {
            let mut entered = BOOL::default();
            if let Some(sender) = sender {
                // SAFETY: the sender is the live webview of this UI-thread event.
                unsafe { sender.ContainsFullScreenElement(&raw mut entered)? };
            }
            on_change(entered.as_bool());
            Ok(())
        }));
    let mut token = 0;
    // SAFETY: UI-thread COM call on the live webview; `WebView2` retains the handler.
    unsafe { core.add_ContainsFullScreenElementChanged(&handler, &raw mut token) }
        .map_err(|e| format!("WebView2 full screen event: {e}"))
}
