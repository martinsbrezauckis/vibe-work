//! macOS: turns WebTransport off in one preview webview. `WebKit` opens WebTransport sessions
//! directly, without the data store's `proxyConfigurations` (public since macOS 26.4; the `CyberInsider` report on the Psylo
//! findings, 2026), so a page could reach this computer or the local network around the network guard's proxy. The
//! API is on by default in secure contexts (measured on macOS 27.0.1: `typeof WebTransport` is
//! `"function"` on an `http://127.0.0.1` page). `WebKit` has no public switch; this uses the feature list of
//! `WKPreferences` (`+_features`, `-_setEnabled:forFeature:`, `-_isEnabledForFeature:`, the SPI behind Safari's
//! Feature Flags) and reads the value back: the caller treats any failure as "not turned off".
//!
//! Depends on: objc2, objc2-foundation, objc2-web-kit (`WKWebView`, `WKPreferences`). Called by: vw-webview
//! `platform::web_transport_off` (inside Tauri's `with_webview`, on the main thread), before the first page.
//! Invariants:
//! - Never assumes the SPI exists: every selector is checked with `respondsToSelector:` first.
//! - Changes only the given webview's preferences object (each preview has its own configuration).
//! - `unsafe` is confined to the platform calls, each with its SAFETY comment.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{ClassType, MainThreadMarker, msg_send, sel};
use objc2_foundation::{NSArray, NSString};
use objc2_web_kit::{WKPreferences, WKWebView};

/// The feature key of WebTransport in `WebKit`'s preference list (`WebPreferences.yaml`).
const FEATURE: &str = "WebTransportEnabled";

/// Turns WebTransport off for `webview` (Tauri's `PlatformWebview::inner()`, a `WKWebView`, on the main thread) and
/// checks that it reads back off. Applies to documents loaded from then on.
///
/// # Errors
/// Not on the main thread, no webview, the preference SPI is missing, the feature is not listed, or it still reads on.
pub fn turn_off_web_transport(webview: *mut c_void) -> Result<(), String> {
    if MainThreadMarker::new().is_none() {
        return Err("WebTransport can be turned off on the main thread only".to_owned());
    }
    if webview.is_null() {
        return Err("the preview webview is not available".to_owned());
    }
    // SAFETY: Tauri hands us its live `WKWebView` pointer on the main thread inside `with_webview`.
    let view = unsafe { &*webview.cast::<WKWebView>() };
    // SAFETY: main-thread property reads; WebKit hands out the preferences object the webview uses.
    let prefs = unsafe { view.configuration().preferences() };
    let class = WKPreferences::class();
    // SAFETY: `respondsToSelector:` is defined on every Objective-C object, class objects included.
    let has: [bool; 3] = unsafe {
        [
            msg_send![class, respondsToSelector: sel!(_features)],
            msg_send![&*prefs, respondsToSelector: sel!(_setEnabled:forFeature:)],
            msg_send![&*prefs, respondsToSelector: sel!(_isEnabledForFeature:)],
        ]
    };
    if has.contains(&false) {
        return Err("this WebKit has no preference feature list".to_owned());
    }
    // SAFETY: checked above; `+[WKPreferences _features]` returns an array of `_WKFeature` objects.
    let features: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![class, _features] };
    let features = features.ok_or("WebKit listed no preference features")?;
    for feature in &features {
        // SAFETY: `_WKFeature.key` is an `NSString` property; checked with `respondsToSelector:` first.
        let key: Option<Retained<NSString>> = unsafe {
            let readable: bool = msg_send![&*feature, respondsToSelector: sel!(key)];
            if readable {
                msg_send![&*feature, key]
            } else {
                None
            }
        };
        if key.is_none_or(|key| key.to_string() != FEATURE) {
            continue;
        }
        // SAFETY: both selectors checked above; the feature object comes from the same list; BOOL is passed as
        // `Bool` (an object here would be read as YES).
        let on: Bool = unsafe {
            let () = msg_send![&*prefs, _setEnabled: Bool::NO, forFeature: &*feature];
            msg_send![&*prefs, _isEnabledForFeature: &*feature]
        };
        return if on.as_bool() {
            Err("WebTransport still reads on after turning it off".to_owned())
        } else {
            Ok(())
        };
    }
    Err(format!("WebKit does not list the {FEATURE} feature"))
}
