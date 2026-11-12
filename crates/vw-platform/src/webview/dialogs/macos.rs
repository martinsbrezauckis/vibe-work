//! `WKWebView` UI/navigation delegate proxies hold native dialog completion blocks on the main thread.
//! Unhandled selectors forward to Wry's retained delegates, preserving navigation, popups, file pickers and downloads.
//! Depends on: Objective-C delegate forwarding and the shared dialog hub.
//! Called by: the macOS webview dialog adapter.
use super::{Admit, Hub, hold};
use block2::Block;
use objc2::{
    DefinedClass, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, NSObject, Sel},
    sel,
};
use objc2_foundation::{
    MainThreadMarker, NSObjectProtocol, NSString, NSURLAuthenticationChallenge, NSURLCredential,
    NSURLCredentialPersistence, NSURLSessionAuthChallengeDisposition,
};
use objc2_web_kit::{WKFrameInfo, WKWebView};
use serde_json::json;
use std::{cell::RefCell, collections::HashMap, sync::Arc};

struct Context {
    original: Option<Retained<AnyObject>>,
    label: String,
    hub: Arc<Hub>,
    admit: Admit,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VibeWorkDialogDelegate"]
    #[ivars = Context]
    struct DialogDelegate;
    unsafe impl NSObjectProtocol for DialogDelegate {}
    impl DialogDelegate {
        #[unsafe(method(respondsToSelector:))]
        fn responds(&self, selector: Sel) -> bool {
            // SAFETY: NSObject's own implementation checks this class before forwarding to the live original.
            let own: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            own || self.original_responds(selector)
        }
        #[unsafe(method(forwardingTargetForSelector:))]
        fn forward(&self, selector: Sel) -> *mut AnyObject {
            if self.original_responds(selector) { self.ivars().original.as_ref().map_or(std::ptr::null_mut(), |v| Retained::as_ptr(v).cast_mut()) } else { std::ptr::null_mut() }
        }
        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn alert(&self, view: &WKWebView, message: &NSString, frame: &WKFrameInfo, done: &Block<dyn Fn()>) {
            if !(self.ivars().admit)() {
                if let Some(original) = self.original(sel!(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:)) {
                    // SAFETY: forwarded with the original WebKit delegate signature and live callback arguments.
                    unsafe { let _: () = msg_send![original, webView:view, runJavaScriptAlertPanelWithMessage:message, initiatedByFrame:frame, completionHandler:done]; }
                } else { done.call(()); }
                return;
            }
            let done = done.copy();
            self.hold("alert", message, None, frame, Box::new(move |_| { done.call(()); Ok(()) }));
        }
        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn confirm(&self, view: &WKWebView, message: &NSString, frame: &WKFrameInfo, done: &Block<dyn Fn(Bool)>) {
            if !(self.ivars().admit)() {
                if let Some(original) = self.original(sel!(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:)) {
                    // SAFETY: same native delegate ABI, with the original callback's lifetime.
                    unsafe { let _: () = msg_send![original, webView:view, runJavaScriptConfirmPanelWithMessage:message, initiatedByFrame:frame, completionHandler:done]; }
                } else { done.call((Bool::NO,)); }
                return;
            }
            let done = done.copy();
            self.hold("confirm", message, None, frame, Box::new(move |answer| { done.call((Bool::new(answer.accept),)); Ok(()) }));
        }
        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn prompt(&self, view: &WKWebView, message: &NSString, default: Option<&NSString>, frame: &WKFrameInfo, done: &Block<dyn Fn(*mut NSString)>) {
            if !(self.ivars().admit)() {
                if let Some(original) = self.original(sel!(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:)) {
                    // SAFETY: same native delegate ABI and nullable default text.
                    unsafe { let _: () = msg_send![original, webView:view, runJavaScriptTextInputPanelWithPrompt:message, defaultText:default, initiatedByFrame:frame, completionHandler:done]; }
                } else { done.call((std::ptr::null_mut(),)); }
                return;
            }
            let done = done.copy();
            self.hold("prompt", message, default, frame, Box::new(move |answer| {
                let text = answer.accept.then(||NSString::from_str(answer.text.as_deref().unwrap_or_default()));
                done.call((text.as_ref().map_or(std::ptr::null_mut(), |v|Retained::as_ptr(v).cast_mut()),)); Ok(())
            }));
        }
        // WebKit's existing macOS SPI; public WKUIDelegate has no before-unload callback.
        #[unsafe(method(_webView:runBeforeUnloadConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn before_unload(&self, view: &WKWebView, message: &NSString, frame: &WKFrameInfo, done: &Block<dyn Fn(Bool)>) {
            if !(self.ivars().admit)() {
                if let Some(original) = self.original(sel!(_webView:runBeforeUnloadConfirmPanelWithMessage:initiatedByFrame:completionHandler:)) {
                    // SAFETY: selector and BOOL completion ABI follow WebKit's WKUIDelegatePrivate declaration.
                    unsafe { let _: () = msg_send![original, _webView:view, runBeforeUnloadConfirmPanelWithMessage:message, initiatedByFrame:frame, completionHandler:done]; }
                } else { done.call((Bool::YES,)); }
                return;
            }
            let done = done.copy();
            self.hold("beforeunload", message, None, frame, Box::new(move |answer| { done.call((Bool::new(answer.accept),)); Ok(()) }));
        }
        #[unsafe(method(webView:didReceiveAuthenticationChallenge:completionHandler:))]
        fn authenticate(&self, view: &WKWebView, challenge: &NSURLAuthenticationChallenge, done: &Block<dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential)>) {
            let space = challenge.protectionSpace();
            if !(self.ivars().admit)() || space.isProxy() || space.authenticationMethod().to_string() != "NSURLAuthenticationMethodHTTPBasic" {
                if let Some(original) = self.original(sel!(webView:didReceiveAuthenticationChallenge:completionHandler:)) {
                    // SAFETY: forwards proxy, TLS and other challenges to the original navigation delegate.
                    unsafe { let _: () = msg_send![original, webView:view, didReceiveAuthenticationChallenge:challenge, completionHandler:done]; }
                } else { done.call((NSURLSessionAuthChallengeDisposition::PerformDefaultHandling, std::ptr::null_mut())); }
                return;
            }
            let scheme = space.protocol().map(|s|s.to_string()).unwrap_or_default();
            let host = space.host().to_string();
            let host = if host.contains(':') { format!("[{host}]") } else {host};
            let origin = url::Url::parse(&format!("{scheme}://{host}:{}",space.port())).map(|u|u.origin().ascii_serialization()).unwrap_or_default();
            let info = json!({"type":"auth","scheme":"basic","message":"HTTP sign-in required","origin":origin,"url":origin,"realm":space.realm().map(|s|s.to_string()),"retry":challenge.previousFailureCount()>0});
            let done = done.copy();
            hold(&self.ivars().hub, &self.ivars().label, info, Box::new(move |answer| {
                if let Some((username,password)) = answer.credentials.filter(|_|answer.accept) {
                    let credential = NSURLCredential::credentialWithUser_password_persistence(&NSString::from_str(&username), &NSString::from_str(&password), NSURLCredentialPersistence::None);
                    done.call((NSURLSessionAuthChallengeDisposition::UseCredential, Retained::as_ptr(&credential).cast_mut()));
                } else { done.call((NSURLSessionAuthChallengeDisposition::CancelAuthenticationChallenge, std::ptr::null_mut())); }
                Ok(())
            }));
        }
    }
);
impl DialogDelegate {
    fn new(context: Context, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(context);
        // SAFETY: initializes a freshly allocated NSObject subclass with its Rust ivars set.
        unsafe { msg_send![super(this), init] }
    }
    fn original_responds(&self, selector: Sel) -> bool {
        self.original(selector).is_some()
    }
    fn original(&self, selector: Sel) -> Option<&AnyObject> {
        self.ivars().original.as_deref().filter(|original| {
            // SAFETY: the retained original is a live Objective-C delegate on this thread.
            unsafe { msg_send![*original, respondsToSelector: selector] }
        })
    }
    fn hold(
        &self,
        kind: &str,
        message: &NSString,
        default: Option<&NSString>,
        frame: &WKFrameInfo,
        finish: super::Finish,
    ) {
        // SAFETY: WebKit supplies the live frame for this delegate callback.
        let url = unsafe { frame.request() }
            .URL()
            .and_then(|u| u.absoluteString())
            .map(|s| s.to_string())
            .unwrap_or_default();
        hold(
            &self.ivars().hub,
            &self.ivars().label,
            json!({"type":kind,"message":message.to_string(),"defaultPrompt":default.map(ToString::to_string),"url":url}),
            finish,
        );
    }
}
type Delegates = (Retained<DialogDelegate>, Retained<DialogDelegate>);
thread_local! { static DELEGATES: RefCell<HashMap<String, Delegates>> = RefCell::new(HashMap::new()); }

/// Installs forwarding delegates on a live `WKWebView` before its first navigation.
///
/// # Errors
/// Called off the main thread or without a view.
pub fn install(view: usize, label: String, hub: Arc<Hub>, admit: Admit) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("Page dialogs need the main thread.")?;
    // SAFETY: Tauri supplies this live WKWebView on its main thread for the duration of this call.
    let view = unsafe { (view as *mut WKWebView).as_ref() }.ok_or("Browser closed.")?;
    // SAFETY: public delegate properties retain their returned object; WebKit holds the replacements weakly.
    unsafe {
        let original = msg_send![view, UIDelegate];
        let ui = DialogDelegate::new(
            Context {
                original,
                label: label.clone(),
                hub: hub.clone(),
                admit: admit.clone(),
            },
            mtm,
        );
        let original = msg_send![view, navigationDelegate];
        let navigation = DialogDelegate::new(
            Context {
                original,
                label: label.clone(),
                hub,
                admit,
            },
            mtm,
        );
        let _: () = msg_send![view, setUIDelegate:&*ui];
        let _: () = msg_send![view, setNavigationDelegate:&*navigation];
        DELEGATES.with(|all| all.borrow_mut().insert(label, (ui, navigation)));
    }
    Ok(())
}
pub(super) fn forget(label: &str) {
    DELEGATES.with(|all| all.borrow_mut().remove(label));
}
