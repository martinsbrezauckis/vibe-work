//! macOS content rules for preview webviews: compiles a `WKContentRuleList` (the JSON from
//! `vw_netguard::rules` or Browser shields) and replaces only its own rule list, so `WebKit` refuses every request of the page whose
//! URL the rules block before it is sent; removes compiled lists whose webview closed. Kept apart from `webview` so the
//! macOS-only code stays in one place.
//!
//! Depends on: objc2, objc2-foundation, objc2-web-kit (`WKContentRuleListStore`, `WKUserContentController`), block2.
//! Called by: vw-webview `rules_macos` and `engine::shields` (inside Tauri's `with_webview` / `run_on_main_thread`, on the main thread).
//! Invariants:
//! - `apply_content_rules` answers its callback exactly once (also on failure).
//! - `unsafe` is confined to the platform calls, each with its SAFETY comment.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, PoisonError};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message as _};
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{
    WKContentRuleList, WKContentRuleListStore, WKUserContentController, WKWebView,
};

type Installed = (
    Retained<WKUserContentController>,
    Retained<WKContentRuleList>,
);

thread_local! {
    // Separate identifiers for network policy and shields: neither update may remove the other's rules.
    static INSTALLED: RefCell<HashMap<String, Installed>> = RefCell::new(HashMap::new());
}

fn replace(
    identifier: &str,
    controller: Retained<WKUserContentController>,
    list: Option<&WKContentRuleList>,
) {
    INSTALLED.with(|installed| {
        let mut installed = installed.borrow_mut();
        if let Some((old_controller, old)) = installed.remove(identifier) {
            // SAFETY: both objects are retained and this helper is called only on the main thread.
            unsafe {
                old_controller.removeContentRuleList(&old);
            }
        }
        if let Some(list) = list {
            // SAFETY: the callback supplies a live list; retain it for the next replacement.
            unsafe {
                controller.addContentRuleList(list);
            }
            installed.insert(identifier.to_owned(), (controller, list.retain()));
        }
    });
}

/// A completion callback for an operation without a value.
pub type UnitDone = Box<dyn FnOnce(Result<(), String>) + Send>;

/// Compiles the content rule list `json` under `identifier` (the default `WKContentRuleListStore`, which keeps
/// it on disk until [`forget_content_rules`]) and replaces its own rule list of `webview` (Tauri's
/// `PlatformWebview::inner()`, a `WKWebView`, on the main thread). Answers `done` once the list is in place (it
/// applies to every request the page makes from then on), or with the compile error.
///
/// `install_if` is asked on the main thread when the compile finished: `false` (a newer list was started for this
/// webview meanwhile) leaves the webview's rules as they are and answers an error.
pub fn apply_content_rules(
    webview: *mut c_void,
    identifier: &str,
    json: &str,
    install_if: Box<dyn Fn() -> bool + Send>,
    done: UnitDone,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        done(Err(
            "content rules must be applied on the main thread".to_owned()
        ));
        return;
    };
    if webview.is_null() {
        done(Err("the preview webview is not available".to_owned()));
        return;
    }
    // SAFETY: Tauri hands us its live `WKWebView` pointer on the main thread inside `with_webview`; retaining it
    // keeps it alive until the compile answers.
    let Some(view) = (unsafe { Retained::retain(webview.cast::<WKWebView>()) }) else {
        done(Err("the preview webview is not available".to_owned()));
        return;
    };
    // SAFETY: called on the main thread (marker above).
    let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(mtm) }) else {
        done(Err("WebKit has no content rule list store".to_owned()));
        return;
    };
    if !install_if() {
        done(Err("a newer rule list was started for this preview".into()));
        return;
    }
    if json.trim() == "[]" {
        // SAFETY: retained live view, on the main thread.
        replace(
            identifier,
            unsafe { view.configuration().userContentController() },
            None,
        );
        done(Ok(()));
        return;
    }
    let key = identifier.to_owned();
    let done = Mutex::new(Some(done));
    let block = RcBlock::new(move |list: *mut WKContentRuleList, error: *mut NSError| {
        let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take() else {
            return;
        };
        // SAFETY: WebKit passes either a valid list or a valid error for the duration of this call, on the main
        // thread.
        let result = match unsafe { list.as_ref() } {
            Some(_) if !install_if() => {
                Err("a newer rule list was started for this preview".to_owned())
            }
            Some(list) => {
                // SAFETY: `view` is retained; its configuration's user content controller is the one the webview
                // uses (WebKit hands out the same controller object).
                unsafe {
                    let controller = view.configuration().userContentController();
                    replace(&key, controller, Some(list));
                }
                Ok(())
            }
            None => Err(unsafe { error.as_ref() }.map_or_else(
                || "the content rules did not compile".to_owned(),
                |e| e.localizedDescription().to_string(),
            )),
        };
        done(result);
    });
    let (identifier, json) = (NSString::from_str(identifier), NSString::from_str(json));
    // SAFETY: valid strings and a block WebKit retains until it calls it (on the main thread).
    unsafe {
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(&identifier),
            Some(&json),
            Some(&*block),
        );
    }
}

/// Removes the compiled rule list `identifier` from the default store (its webview closed). Call on the main
/// thread; a missing list is fine.
pub fn forget_content_rules(identifier: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    INSTALLED.with(|installed| {
        installed.borrow_mut().remove(identifier);
    });
    // SAFETY: called on the main thread (marker above).
    let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(mtm) }) else {
        return;
    };
    let block = RcBlock::new(|_error: *mut NSError| {});
    let identifier = NSString::from_str(identifier);
    // SAFETY: a valid string and a block WebKit retains until it calls it.
    unsafe {
        store
            .removeContentRuleListForIdentifier_completionHandler(Some(&identifier), Some(&*block));
    }
}
