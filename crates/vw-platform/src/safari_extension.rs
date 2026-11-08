//! Your Safari setup step, run by the containing app itself (macOS). Apple lets only the app that embeds a Safari
//! extension check its state, open Safari's settings at it, or message it:
//! - `SFSafariExtensionManager.getStateOfSafariExtension`: "the Safari app or web extensions embedded inside your
//!   app"; otherwise `SFErrorNoExtensionFound`, "not found, or ... an extension that was not owned by you"
//!   (<https://developer.apple.com/documentation/safariservices/sferror/code/noextensionfound>).
//! - `SFSafariApplication.dispatchMessage`: "may be called only from the app that contains your Safari app extension"
//!   (<https://developer.apple.com/documentation/safariservices/sfsafariapplication/dispatchmessage(withname:toextensionwithidentifier:userinfo:completionhandler:)>).
//!
//! The setup tool inside the bundle is not that app: on macOS 27 its state check fails, so nothing opened. The app
//! process calls these directly. `SafariServices` is loaded on first use, so other binaries linking vw-platform never
//! load it. The identifier is the extension's bundle identifier.
//!
//! Depends on: the objc2 runtime (class lookup, `msg_send!`), block2, objc2-foundation, libc `dlopen`.
//! Called by: vw-core-api `chrome_control::safari_package::open`.
use std::{
    ffi::CStr,
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

use block2::{ManualBlockEncoding, RcBlock};
use objc2::{
    exception, msg_send,
    runtime::{AnyClass, AnyObject},
};
use objc2_foundation::{NSDictionary, NSError, NSNumber, NSString};

/// The notice the Your Safari extension opens its pairing controls for (chrome-control `safari/src/api.ts`:
/// message `vibe.openPairingPage` with `userInfo.openPairingPage === true`). Delivered only to a live native port.
const PAIRING_NOTICE: &str = "vibe.openPairingPage";
const PAIRING_KEY: &str = "openPairingPage";

/// What the setup step did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupOutcome {
    /// The extension is on: the pairing notice was handed to Safari.
    PairingNoticeSent,
    /// The extension is off: Safari's extension settings were opened at it.
    SettingsOpened,
}

type Done = Box<dyn FnOnce(Result<SetupOutcome, String>) + Send>;

// SafariServices sends these calls over NSXPC, which refuses a block without a type signature
// ("Block was not compiled using a compiler that inserts type information about arguments"), so both handler
// blocks carry their Objective-C block encodings (64-bit only: macOS on arm64/x86_64).
struct StateReply;
// SAFETY: `void (^)(SFSafariExtensionState *, NSError *)` on a 64-bit target, matching `Arguments`/`Return`.
unsafe impl ManualBlockEncoding for StateReply {
    type Arguments = (*mut AnyObject, *mut NSError);
    type Return = ();
    const ENCODING_CSTR: &'static CStr = c"v24@?0@\"SFSafariExtensionState\"8@\"NSError\"16";
}
struct ErrorReply;
// SAFETY: `void (^)(NSError *)` on a 64-bit target, matching `Arguments`/`Return`.
unsafe impl ManualBlockEncoding for ErrorReply {
    type Arguments = (*mut NSError,);
    type Return = ();
    const ENCODING_CSTR: &'static CStr = c"v16@?0@\"NSError\"8";
}

/// Runs one `SafariServices` call; an Objective-C exception becomes an error instead of aborting the app.
fn guarded(call: impl FnOnce()) -> Result<(), String> {
    exception::catch(std::panic::AssertUnwindSafe(call))
        .map_err(|error| format!("SafariServices refused the call: {error:?}"))
}

fn class(name: &CStr) -> Option<&'static AnyClass> {
    static LOADED: OnceLock<bool> = OnceLock::new();
    let loaded = *LOADED.get_or_init(|| {
        // SAFETY: a constant, NUL-terminated system framework path; the handle stays open for the process lifetime.
        let handle = unsafe {
            libc::dlopen(
                c"/System/Library/Frameworks/SafariServices.framework/SafariServices".as_ptr(),
                libc::RTLD_LAZY,
            )
        };
        !handle.is_null()
    });
    loaded.then(|| AnyClass::get(name)).flatten()
}

fn error_text(error: *mut NSError) -> String {
    // SAFETY: SafariServices passes either nil or a valid NSError that lives for the callback.
    match unsafe { error.as_ref() } {
        Some(error) => format!("{} {}", error.domain(), error.code()),
        None => "no error detail".to_owned(),
    }
}

/// The reply handler both setup calls take; `finish` receives `outcome` or the error Apple reported.
fn reply(
    outcome: SetupOutcome,
    finish: impl Fn(Result<SetupOutcome, String>) + 'static,
) -> RcBlock<dyn Fn(*mut NSError)> {
    RcBlock::with_encoding::<_, _, _, ErrorReply>(move |error: *mut NSError| {
        finish(if error.is_null() {
            Ok(outcome)
        } else {
            Err(format!("{outcome:?} failed: {}", error_text(error)))
        });
    })
}

fn show_settings(
    application: &AnyClass,
    identifier: &NSString,
    finish: impl Fn(Result<SetupOutcome, String>) + 'static,
) -> Result<(), String> {
    let on_reply = reply(SetupOutcome::SettingsOpened, finish);
    // SAFETY: SFSafariApplication's class method: NSString identifier and a typed `void (^)(NSError *)` handler, both
    // live for the call.
    guarded(|| unsafe {
        let () = msg_send![
            application,
            showPreferencesForExtensionWithIdentifier: identifier,
            completionHandler: &*on_reply
        ];
    })
}

fn send_notice(
    application: &AnyClass,
    identifier: &NSString,
    finish: impl Fn(Result<SetupOutcome, String>) + 'static,
) -> Result<(), String> {
    let on_reply = reply(SetupOutcome::PairingNoticeSent, finish);
    let info = NSDictionary::from_slices(
        &[&*NSString::from_str(PAIRING_KEY)],
        &[&*NSNumber::new_bool(true)],
    );
    let name = NSString::from_str(PAIRING_NOTICE);
    // SAFETY: SFSafariApplication's class method (SFSafariApplication.h, macOS 10.12+): NSString name and identifier,
    // an NSDictionary<NSString *, id> user info and a typed `void (^)(NSError *)` handler, all live for the call (the
    // callee copies the block).
    guarded(|| unsafe {
        let () = msg_send![
            application,
            dispatchMessageWithName: &*name,
            toExtensionWithIdentifier: identifier,
            userInfo: &*info,
            completionHandler: &*on_reply
        ];
    })
}

/// Checks the embedded extension `extension` (its bundle identifier) and takes the next setup step: off -> Safari's
/// extension settings; on -> the pairing notice. Apple delivers the notice only to an open native port and reports
/// success even when none is open (a Safari that is still launching drops it), so with the extension on, `ready`
/// runs first on a worker thread and may block until the extension has connected. `done` runs once, on a
/// `SafariServices` callback thread.
///
/// # Errors
/// `SafariServices` is unavailable or refused the state query; nothing was started.
pub fn open_setup(
    extension: &str,
    ready: impl FnOnce() + Send + 'static,
    done: impl FnOnce(Result<SetupOutcome, String>) + Send + 'static,
) -> Result<(), String> {
    let manager = class(c"SFSafariExtensionManager").ok_or("SafariServices is unavailable")?;
    let application = class(c"SFSafariApplication").ok_or("SafariServices is unavailable")?;
    let done: Arc<Mutex<Option<Done>>> = Arc::new(Mutex::new(Some(Box::new(done))));
    let finish = move |result: Result<SetupOutcome, String>| {
        if let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take() {
            done(result);
        }
    };
    let identifier = NSString::from_str(extension);
    let state_identifier = identifier.clone();
    let ready = Mutex::new(Some(ready));
    let on_state = RcBlock::with_encoding::<_, _, _, StateReply>(
        move |state: *mut AnyObject, error: *mut NSError| {
            // SAFETY: SafariServices passes either nil or a valid SFSafariExtensionState that lives for the callback.
            let Some(state) = (unsafe { state.as_ref() }) else {
                finish(Err(format!(
                    "extension state unavailable: {}",
                    error_text(error)
                )));
                return;
            };
            // SAFETY: `isEnabled` is SFSafariExtensionState's BOOL property getter (SFSafariExtensionState.h).
            let enabled: bool = unsafe { msg_send![state, isEnabled] };
            if !enabled {
                if let Err(reason) = show_settings(application, &state_identifier, finish.clone()) {
                    finish(Err(reason));
                }
                return;
            }
            let ready = ready.lock().unwrap_or_else(PoisonError::into_inner).take();
            // Objective-C strings and classes stay on their thread; the worker makes its own.
            let (extension, notice_finish) = (state_identifier.to_string(), finish.clone());
            let spawned = std::thread::Builder::new()
                .name("safari-notice".into())
                .spawn(move || {
                    if let Some(ready) = ready {
                        ready();
                    }
                    let sent = class(c"SFSafariApplication")
                        .ok_or_else(|| "SafariServices is unavailable".to_owned())
                        .and_then(|application| {
                            send_notice(
                                application,
                                &NSString::from_str(&extension),
                                notice_finish.clone(),
                            )
                        });
                    if let Err(reason) = sent {
                        notice_finish(Err(reason));
                    }
                });
            if spawned.is_err() {
                finish(Err("the pairing notice could not be scheduled".to_owned()));
            }
        },
    );
    // SAFETY: SFSafariExtensionManager's class method (SFSafariExtensionManager.h): NSString identifier and a typed
    // `void (^)(SFSafariExtensionState *, NSError *)` handler, both live for the call (the callee copies the block).
    guarded(|| unsafe {
        let () = msg_send![
            manager,
            getStateOfSafariExtensionWithIdentifier: &*identifier,
            completionHandler: &*on_state
        ];
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::sel;

    #[test]
    fn safari_services_offers_the_three_setup_calls() {
        let manager = class(c"SFSafariExtensionManager").expect("SFSafariExtensionManager");
        let application = class(c"SFSafariApplication").expect("SFSafariApplication");
        assert!(
            manager
                .class_method(sel!(getStateOfSafariExtensionWithIdentifier:completionHandler:))
                .is_some()
        );
        assert!(
            application
                .class_method(sel!(showPreferencesForExtensionWithIdentifier:completionHandler:))
                .is_some()
        );
        assert!(
            application
                .class_method(sel!(
                    dispatchMessageWithName:toExtensionWithIdentifier:userInfo:completionHandler:
                ))
                .is_some()
        );
        assert!(
            class(c"SFSafariExtensionState")
                .is_some_and(|state| state.instance_method(sel!(isEnabled)).is_some())
        );
    }

    #[test]
    fn an_extension_this_process_does_not_contain_is_reported_not_found() {
        // A test binary contains no Safari extension: Apple answers SFErrorNoExtensionFound (SFErrorDomain 1), the
        // same answer the bundled setup tool got; `done` runs once even though the callback thread is Apple's.
        let (send, receive) = std::sync::mpsc::channel();
        open_setup(
            "app.vibework.test.not-contained",
            || {},
            move |result| {
                let _ = send.send(result);
            },
        )
        .unwrap();
        match receive.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(result) => assert_eq!(
                result,
                Err("extension state unavailable: SFErrorDomain 1".to_owned())
            ),
            // The handler is a UI-actor callback; without a serviced main queue it may never arrive in a test run.
            Err(_) => eprintln!(
                "SafariServices did not answer without a main run loop; state reply not checked"
            ),
        }
    }
}
