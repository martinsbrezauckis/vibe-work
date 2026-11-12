//! The agent browser's request watch on macOS through Apple's PRIVATE `-[WKWebView _setResourceLoadDelegate:]`
//! (`_WKResourceLoadDelegate`, `WebKit` `WKWebViewPrivate.h`; present since macOS 11). Watch only: every request of the
//! page (documents, frames, scripts, images, fetch/XHR), bounded headers, redirects, response status and completion; no bodies,
//! no changes, no cancelling.
//!
//! Private API: not allowed in an App Store build; legal for a notarized direct download. Any macOS update may
//! change or remove it, so required selectors are checked at run time ([`attach`] answers `false`). The
//! browser log surfaces that failure with a diagnosis and retry instruction. The separate developer fixture
//! can opt into its own watch. Re-test this interface on each macOS release.
//!
//! Depends on: objc2, objc2-foundation, objc2-web-kit. Called by: vw-webview `mac_agent` (inside Tauri's
//! `with_webview`, on the main thread).
//! Invariants: main thread only; `WebKit` holds the delegate weakly, so it is kept here per webview until [`detach`];
//! the sink is called on the main thread and must return quickly.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSDictionary, NSError, NSObjectProtocol, NSString, NSURL};
use objc2_web_kit::WKWebView;

pub use super::request_event::{HeaderFields, RequestEvent, RequestSink, RequestStep};

/// The delegate's sink.
struct Watch {
    sink: RequestSink,
}

define_class!(
    /// Turns `_WKResourceLoadDelegate` calls into [`RequestEvent`]s.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VibeWorkRequestWatch"]
    #[ivars = Watch]
    struct WatchDelegate;

    unsafe impl NSObjectProtocol for WatchDelegate {}

    impl WatchDelegate {
        #[unsafe(method(webView:resourceLoad:didSendRequest:))]
        fn did_send(&self, _view: &AnyObject, load: &AnyObject, request: &AnyObject) {
            let url = request_url(request);
            self.emit(load, RequestStep::Sent, url, None, None, Some(request));
        }

        #[unsafe(method(webView:resourceLoad:didPerformHTTPRedirection:newRequest:))]
        fn did_redirect(
            &self,
            _view: &AnyObject,
            load: &AnyObject,
            response: &AnyObject,
            request: &AnyObject,
        ) {
            let url = request_url(request);
            self.emit(load, RequestStep::Redirected, url, Some(response), None, Some(request));
        }

        #[unsafe(method(webView:resourceLoad:didReceiveResponse:))]
        fn did_receive(&self, _view: &AnyObject, load: &AnyObject, response: &AnyObject) {
            let url = response_url(response);
            self.emit(load, RequestStep::Response, url, Some(response), None, None);
        }

        #[unsafe(method(webView:resourceLoad:didCompleteWithError:response:))]
        fn did_complete(
            &self,
            _view: &AnyObject,
            load: &AnyObject,
            error: *mut NSError,
            response: *mut AnyObject,
        ) {
            // SAFETY: WebKit passes a valid response/error or nil for the duration of this call.
            let (response, error) = unsafe { (response.as_ref(), error.as_ref()) };
            let url = response.map(response_url).unwrap_or_default();
            let error = error.map(|e| e.localizedDescription().to_string());
            self.emit(load, RequestStep::Completed, url, response, error, None);
        }
    }
);

impl WatchDelegate {
    fn new(mtm: MainThreadMarker, sink: RequestSink) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Watch { sink });
        // SAFETY: `init` on a freshly allocated NSObject subclass.
        unsafe { msg_send![super(this), init] }
    }

    fn emit(
        &self,
        load: &AnyObject,
        step: RequestStep,
        url: String,
        response: Option<&AnyObject>,
        error: Option<String>,
        request: Option<&AnyObject>,
    ) {
        let info = LoadInfo::read(load);
        let url = if url.is_empty() { info.url } else { url };
        let status = response.map_or(0, status_of);
        (self.ivars().sink)(RequestEvent {
            id: info.id,
            step,
            url,
            response_url: response.map(response_url),
            method: request.map_or(info.method, |request| {
                // SAFETY: WebKit supplies an NSURLRequest with its documented HTTPMethod getter.
                let method: Option<Retained<NSString>> = unsafe { msg_send![request, HTTPMethod] };
                method.map(|method| method.to_string()).unwrap_or_default()
            }),
            resource_type: info.resource_type,
            status,
            from_cache: info.from_cache,
            main_document: info.main_document,
            request_headers: request.and_then(|r| headers(r, sel!(allHTTPHeaderFields))),
            response_headers: response.and_then(|r| headers(r, sel!(allHeaderFields))),
            error,
        });
    }
}

/// The `_WKResourceLoadInfo` fields this watch reads, each only when the object answers its selector.
struct LoadInfo {
    id: u64,
    url: String,
    method: String,
    resource_type: i64,
    from_cache: bool,
    main_document: bool,
}

impl LoadInfo {
    fn read(load: &AnyObject) -> Self {
        let responds = |s| -> bool {
            // SAFETY: `respondsToSelector:` on a live object.
            unsafe { msg_send![load, respondsToSelector: s] }
        };
        // SAFETY: each getter is sent only when the object answers it; the types follow `_WKResourceLoadInfo.h`
        // (uint64_t, NSURL *, NSString *, NSInteger enum, BOOL).
        unsafe {
            let id: u64 = if responds(sel!(resourceLoadID)) {
                msg_send![load, resourceLoadID]
            } else {
                0
            };
            let url = if responds(sel!(originalURL)) {
                let url: Option<Retained<NSURL>> = msg_send![load, originalURL];
                url.and_then(|u| u.absoluteString())
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let method = if responds(sel!(originalHTTPMethod)) {
                let m: Option<Retained<NSString>> = msg_send![load, originalHTTPMethod];
                m.map(|m| m.to_string()).unwrap_or_default()
            } else {
                String::new()
            };
            let resource_type: isize = if responds(sel!(resourceType)) {
                msg_send![load, resourceType]
            } else {
                -1
            };
            let from_cache: bool = if responds(sel!(loadedFromCache)) {
                msg_send![load, loadedFromCache]
            } else {
                false
            };
            let main_document = resource_type == 3 && responds(sel!(parentFrame)) && {
                let parent: Option<Retained<AnyObject>> = msg_send![load, parentFrame];
                parent.is_none()
            };
            Self {
                id,
                url,
                method,
                resource_type: resource_type as i64,
                from_cache,
                main_document,
            }
        }
    }
}

/// Copies at most 32 native header fields before they cross into the request log.
fn headers(object: &AnyObject, selector: objc2::runtime::Sel) -> Option<HeaderFields> {
    // SAFETY: WebKit supplies NSURLRequest/NSURLResponse objects; only request/HTTP response
    // selectors returning an NSDictionary of string header fields are passed by our callbacks.
    let dictionary: Option<Retained<NSDictionary<NSString, NSString>>> = unsafe {
        let responds: bool = msg_send![object, respondsToSelector: selector];
        if !responds {
            return None;
        }
        if selector == sel!(allHTTPHeaderFields) {
            msg_send![object, allHTTPHeaderFields]
        } else {
            msg_send![object, allHeaderFields]
        }
    };
    let Some(dictionary) = dictionary else {
        return Some(HeaderFields::default());
    };
    let mut fields = HeaderFields {
        truncated: dictionary.len() > 32,
        ..HeaderFields::default()
    };
    for key in dictionary.keys().take(32) {
        if let Some(value) = dictionary.objectForKey(&key) {
            let (key, value) = (key.to_string(), value.to_string());
            fields.truncated |= key.chars().count() > 128 || value.chars().count() > 512;
            fields.values.insert(
                key.chars().take(128).collect(),
                value.chars().take(512).collect(),
            );
        }
    }
    Some(fields)
}

fn request_url(request: &AnyObject) -> String {
    // SAFETY: an NSURLRequest from WebKit; `URL` is its documented getter.
    unsafe {
        let url: Option<Retained<NSURL>> = msg_send![request, URL];
        url.and_then(|u| u.absoluteString())
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
}

fn response_url(response: &AnyObject) -> String {
    // SAFETY: an NSURLResponse from WebKit; `URL` is its documented getter.
    unsafe {
        let url: Option<Retained<NSURL>> = msg_send![response, URL];
        url.and_then(|u| u.absoluteString())
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
}

fn status_of(response: &AnyObject) -> i64 {
    let Some(http) = AnyClass::get(c"NSHTTPURLResponse") else {
        return 0;
    };
    // SAFETY: class membership is checked before the NSHTTPURLResponse getter is sent.
    unsafe {
        let is_http: bool = msg_send![response, isKindOfClass: http];
        if !is_http {
            return 0;
        }
        let code: isize = msg_send![response, statusCode];
        code as i64
    }
}

thread_local! {
    /// Delegates by webview address (WebKit keeps only a weak reference).
    static WATCHES: RefCell<HashMap<usize, Retained<WatchDelegate>>> = RefCell::new(HashMap::new());
}

/// True when this macOS still has the private interface this watch needs. `VW_DEV_REQUEST_WATCH_EXTRA_SELECTOR`
/// names one more webview selector to require (the private engine fixture checks a missing one with it: the same
/// path a macOS update that removes the interface takes).
#[must_use]
pub fn available(webview: *mut c_void) -> bool {
    // SAFETY: Tauri hands over its live `WKWebView` on the main thread inside `with_webview`.
    let Some(view) = (unsafe { webview.cast::<WKWebView>().as_ref() }) else {
        return false;
    };
    let extra = std::env::var("VW_DEV_REQUEST_WATCH_EXTRA_SELECTOR")
        .ok()
        .and_then(|name| std::ffi::CString::new(name).ok());
    if let Some(name) = extra
        && !view.respondsToSelector(objc2::runtime::Sel::register(&name))
    {
        return false;
    }
    view.respondsToSelector(sel!(_setResourceLoadDelegate:))
        && view.respondsToSelector(sel!(_resourceLoadDelegate))
        && AnyClass::get(c"_WKResourceLoadInfo").is_some_and(|class| {
            // SAFETY: an instance-selector query on a class object.
            [
                sel!(resourceLoadID),
                sel!(resourceType),
                sel!(parentFrame),
                sel!(originalURL),
                sel!(originalHTTPMethod),
            ]
            .into_iter()
            .all(|selector| unsafe { msg_send![class, instancesRespondToSelector: selector] })
        })
}

/// Starts watching `webview`'s requests; `false` (and nothing installed) when the private interface is missing.
#[must_use]
pub fn attach(webview: *mut c_void, sink: RequestSink) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    if !available(webview) {
        return false;
    }
    // SAFETY: checked non-null and live in `available`.
    let view = unsafe { &*webview.cast::<WKWebView>() };
    let delegate = WatchDelegate::new(mtm, sink);
    // SAFETY: the selector exists (checked); the delegate implements the protocol's optional methods it needs and is
    // kept alive in `WATCHES` (WebKit's reference is weak).
    unsafe {
        let _: () = msg_send![view, _setResourceLoadDelegate: &*delegate];
    }
    WATCHES.with(|w| w.borrow_mut().insert(webview as usize, delegate));
    true
}

/// Stops watching `webview` (also called when it closes).
pub fn detach(webview: *mut c_void) {
    let had = WATCHES.with(|w| w.borrow_mut().remove(&(webview as usize)));
    if had.is_none() {
        return;
    }
    // SAFETY: Tauri hands over its live `WKWebView`; clearing a delegate we set, with the checked selector.
    if let Some(view) = unsafe { webview.cast::<WKWebView>().as_ref() }
        && view.respondsToSelector(sel!(_setResourceLoadDelegate:))
    {
        // SAFETY: as above; nil clears the delegate.
        unsafe {
            let _: () = msg_send![view, _setResourceLoadDelegate: std::ptr::null::<AnyObject>()];
        }
    }
}
