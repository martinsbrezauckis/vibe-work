//! macOS "Send to Vibe Work": the two services the app declares in its `Info.plist` ("Send to Vibe Work" for files
//! and folders in Finder, "Ask Vibe Work" for selected text in any app), turned on or off in the system's Services
//! list, and the services provider that hands their requests to the app.
//!
//! On and off: the system keeps each service's state in the `pbs` preferences (`NSServicesStatus`, the list System
//! Settings > Keyboard > Keyboard Shortcuts > Services edits); [`set_enabled`] writes both entries there and asks
//! the system to read the services again. The services are declared without `NSRequiredContext`, so they stay off
//! until the user turns them on.
//!
//! Depends on: objc2 (the provider class), objc2-app-kit (`NSApplication`, `NSPasteboard`, `NSWorkspace`),
//! objc2-foundation (`NSUserDefaults`), the `defaults` and `pbs` programs of macOS. Called by: `send_to`, the app's
//! setup (`register_provider`, on the main thread).
//! Invariants: a request only reaches [`register_provider`]'s callback (which adds chips); the provider never
//! answers with data and never runs anything.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardTypeString,
    NSUpdateDynamicServices, NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSUserDefaults};

use super::SendToError;

/// The app's bundle identifier (`tauri.conf.json`).
const BUNDLE: &str = "app.vibework.desktop";
/// `(menu item, message)` of each service, as declared in `Info.plist`.
const SERVICES: [(&str, &str); 2] = [
    ("Send to Vibe Work", "sendFiles"),
    ("Ask Vibe Work", "askText"),
];

/// What a service request hands over: the files and folders, or the text and the app it was selected in.
pub enum ServiceRequest {
    /// Finder's selection.
    Files(Vec<PathBuf>),
    /// Selected text, and the name of the app that was in front.
    Text {
        /// The selection.
        text: String,
        /// The app it came from, when known.
        from: Option<String>,
    },
}

type Deliver = Box<dyn Fn(ServiceRequest) + Send + Sync>;
static DELIVER: OnceLock<Deliver> = OnceLock::new();

fn status_key(item: &str, message: &str) -> String {
    format!("{BUNDLE} - {item} - {message}")
}

/// Whether the services are on in the system's list (both must be; an entry never written counts as off).
#[must_use]
pub fn enabled() -> bool {
    let suite = NSString::from_str("pbs");
    let Some(defaults) = NSUserDefaults::initWithSuiteName(NSUserDefaults::alloc(), Some(&suite))
    else {
        return false;
    };
    let Some(status) = defaults.dictionaryForKey(&NSString::from_str("NSServicesStatus")) else {
        return false;
    };
    SERVICES.iter().all(|(item, message)| {
        status
            .objectForKey(&NSString::from_str(&status_key(item, message)))
            .and_then(|entry| entry.downcast::<NSDictionary>().ok())
            .and_then(|entry| {
                let flag = NSString::from_str("enabled_context_menu");
                entry.objectForKey(flag.as_ref())
            })
            .and_then(|flag| flag.downcast::<NSNumber>().ok())
            .is_some_and(|flag| flag.boolValue())
    })
}

/// Turns both services on or off in the system's list and asks the system to read the list again.
///
/// # Errors
/// [`SendToError::Write`] when `defaults` refused the change.
pub fn set_enabled(on: bool) -> Result<(), SendToError> {
    let flag = u8::from(on);
    for (item, message) in SERVICES {
        let value = format!(
            "{{ enabled_context_menu = {flag}; enabled_services_menu = {flag}; presentation_modes = \
             {{ ContextMenu = {flag}; ServicesMenu = {flag}; }}; }}"
        );
        let output = Command::new("/usr/bin/defaults")
            .args(["write", "pbs", "NSServicesStatus", "-dict-add"])
            .arg(format!("\"{}\"", status_key(item, message)))
            .arg(value)
            .output()
            .map_err(|error| SendToError::Write(error.to_string()))?;
        if !output.status.success() {
            return Err(SendToError::Write(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
    }
    // The services daemon reads the list again; the menus change without logging out.
    let _ = Command::new("/System/Library/CoreServices/pbs")
        .arg("-update")
        .output();
    NSUpdateDynamicServices();
    Ok(())
}

define_class!(
    /// The object macOS sends the two service messages to.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VibeWorkServicesProvider"]
    struct Provider;

    impl Provider {
        /// "Send to Vibe Work": file URLs of Finder's selection.
        #[unsafe(method(sendFiles:userData:error:))]
        fn send_files(
            &self,
            pasteboard: &NSPasteboard,
            _user_data: Option<&NSString>,
            _error: *mut *mut NSString,
        ) {
            let paths: Vec<PathBuf> = pasteboard
                .pasteboardItems()
                .map(|items| {
                    items
                        .iter()
                        // SAFETY: `NSPasteboardTypeFileURL` is an AppKit constant valid for the process's life.
                        .filter_map(|item| item.stringForType(unsafe { NSPasteboardTypeFileURL }))
                        .filter_map(|text| url::Url::parse(&text.to_string()).ok())
                        .filter_map(|url| url.to_file_path().ok())
                        .collect()
                })
                .unwrap_or_default();
            if let Some(deliver) = DELIVER.get() {
                deliver(ServiceRequest::Files(paths));
            }
        }

        /// "Ask Vibe Work": the selected text.
        #[unsafe(method(askText:userData:error:))]
        fn ask_text(
            &self,
            pasteboard: &NSPasteboard,
            _user_data: Option<&NSString>,
            _error: *mut *mut NSString,
        ) {
            // SAFETY: `NSPasteboardTypeString` is an AppKit constant valid for the process's life.
            let Some(text) = pasteboard.stringForType(unsafe { NSPasteboardTypeString }) else {
                return;
            };
            let from = NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .and_then(|app| app.localizedName())
                .map(|name| name.to_string())
                .filter(|name| name != "Vibe Work");
            if let Some(deliver) = DELIVER.get() {
                deliver(ServiceRequest::Text {
                    text: text.to_string(),
                    from,
                });
            }
        }
    }
);

/// Makes this app the provider of its two services (once, on the main thread, after the app object exists);
/// `deliver` receives each request. False when not called on the main thread or already registered.
pub fn register_provider(deliver: impl Fn(ServiceRequest) + Send + Sync + 'static) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    if DELIVER.set(Box::new(deliver)).is_err() {
        return false;
    }
    // SAFETY: a plain `init` of an `NSObject` subclass without instance variables, on the main thread.
    let provider: Retained<Provider> = unsafe { msg_send![Provider::alloc(mtm), init] };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: the provider is a valid object; the application does not retain it, so it is kept for the
    // process's life below.
    unsafe { app.setServicesProvider(Some(AsRef::<AnyObject>::as_ref(&*provider))) };
    std::mem::forget(provider);
    NSUpdateDynamicServices();
    true
}
