//! One `WKWebView`'s native appearance, called synchronously on Tauri's UI thread.
//! Depends on `AppKit`'s per-view appearance; `None` restores appearance inheritance.
use objc2_app_kit::{NSAppearance, NSAppearanceCustomization};
use objc2_foundation::ns_string;
use objc2_web_kit::WKWebView;

/// Applies a light/dark appearance to a live native browser view.
///
/// # Errors
/// The view or named native appearance is unavailable.
pub fn set_color_scheme(view: *mut std::ffi::c_void, dark: Option<bool>) -> Result<(), String> {
    // SAFETY: Tauri supplies its live WKWebView on the UI thread for this synchronous call.
    let view = unsafe { view.cast::<WKWebView>().as_ref() }
        .ok_or("Browser view is unavailable; reopen the tab.")?;
    let appearance = dark
        .map(|dark| {
            NSAppearance::appearanceNamed(if dark {
                ns_string!("NSAppearanceNameDarkAqua")
            } else {
                ns_string!("NSAppearanceNameAqua")
            })
            .ok_or_else(|| {
                "The native appearance is unavailable; use system colorScheme.".to_owned()
            })
        })
        .transpose()?;
    view.setAppearance(appearance.as_deref());
    Ok(())
}
