//! `WebView2`'s native request barrier and ad filter. The browser supplies resource type and top-page URL.
//! Called once per Browser webview before navigation; page JavaScript cannot intercept these decisions.
use super::{DecideRequest, PostToUi, Verdict};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET,
    COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL, ICoreWebView2_22, ICoreWebView2Controller,
    ICoreWebView2Deferral, ICoreWebView2Environment, ICoreWebView2WebResourceRequestedEventArgs,
};
use webview2_com::{NavigationStartingEventHandler, WebResourceRequestedEventHandler, take_pwstr};
use windows::core::{HSTRING, Interface, PWSTR};

/// Native ad decision: request URL, `WebKit` resource type, top-page URL, top-level document.
/// Returns true to block. This is independent of the navigation/network safety verdict.
pub type BlockRequest = Box<dyn Fn(&str, &str, &str, bool) -> bool + Send + Sync>;

/// `WebView2` objects of one held request, moved to a helper thread and back. Only ever dereferenced on the UI
/// thread: built in the handler there, used again only inside a closure [`PostToUi`] runs there.
struct Held {
    args: ICoreWebView2WebResourceRequestedEventArgs,
    deferral: ICoreWebView2Deferral,
    env: ICoreWebView2Environment,
}
// SAFETY: see `Held`: the COM pointers cross threads only as opaque values and are touched on the UI thread.
unsafe impl Send for Held {}

/// Answers `args` with `403` (on the UI thread).
///
/// # Safety
/// Call on the UI thread with live event args and the webview's environment.
unsafe fn refuse(
    env: &ICoreWebView2Environment,
    args: &ICoreWebView2WebResourceRequestedEventArgs,
) -> windows::core::Result<()> {
    // SAFETY: the caller's contract (UI thread, live objects).
    unsafe {
        let refused =
            env.CreateWebResourceResponse(None, 403, &HSTRING::from("Forbidden"), &HSTRING::new())?;
        args.SetResponse(&refused)
    }
}

/// Answers every request the webview sends with `403` before it is sent when the rule refuses it: document
/// requests (main frame and frames) and every other request (scripts, images, `fetch`, media, ...) ask `decide`
/// (call on the UI thread, once per webview, before its first navigation). A [`Verdict::Later`] holds the request
/// with a deferral and completes it on the UI thread through `post` when the verdict arrives, so a slow name
/// lookup never freezes the window.
///
/// # Errors
/// A text reason when the filter or the handler cannot be registered: the caller must not show the webview.
pub fn guard_requests(
    controller: &ICoreWebView2Controller,
    environment: &ICoreWebView2Environment,
    decide: DecideRequest,
    block: BlockRequest,
    post: PostToUi,
) -> Result<(), String> {
    let env = environment.clone();
    let top_url = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let navigating = top_url.clone();
    let navigation = NavigationStartingEventHandler::create(Box::new(move |_, args| {
        if let Some(args) = args {
            let mut uri = PWSTR::null();
            // SAFETY: live top-level navigation arguments, on the UI thread; take_pwstr frees the result.
            unsafe {
                args.Uri(&raw mut uri)?;
            }
            *navigating.borrow_mut() = take_pwstr(uri);
        }
        Ok(())
    }));
    let handler = WebResourceRequestedEventHandler::create(Box::new(move |sender, args| {
        let Some(args) = args else {
            return Ok(());
        };
        // SAFETY: WebView2 hands over live event args for the duration of this call on the UI thread; the
        // URI string is freed by `take_pwstr`; the response is created by the webview's own environment; a
        // deferral keeps the args alive until it is completed (on the UI thread, through `post`).
        unsafe {
            // Every handler sees every request that matches ANY filter of the webview (wry's custom-protocol
            // filters too: `ipc.localhost`, `tauri.localhost`); the rules refuse those for a preview page (it has
            // no IPC rights anyway).
            let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT::default();
            args.ResourceContext(&raw mut context)?;
            let mut uri = PWSTR::null();
            let request = args.Request()?;
            request.Uri(&raw mut uri)?;
            let uri = take_pwstr(uri);
            let mut source = PWSTR::null();
            if let Some(sender) = sender {
                let _ = sender.Source(&raw mut source);
            }
            let source = take_pwstr(source);
            // Source tracks committed documents and same-document URL changes. A pending navigation
            // must not change the origin of the old page's requests if that navigation is cancelled.
            let page = if source.is_empty() || source == "about:blank" {
                top_url.borrow().clone()
            } else {
                source
            };
            let mut destination = PWSTR::null();
            // Sec- headers are browser-owned. Older runtimes may omit these at this event; then
            // the top-level NavigationStarting URL identifies the exempt navigation (including redirects).
            let destination = if request
                .Headers()
                .and_then(|headers| {
                    headers.GetHeader(&HSTRING::from("Sec-Fetch-Dest"), &raw mut destination)
                })
                .is_ok()
            {
                take_pwstr(destination)
            } else {
                String::new()
            };
            let document = context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT;
            let top = document
                && (destination == "document"
                    || (destination.is_empty()
                        && (uri == *top_url.borrow()
                            || uri == page
                            || page.is_empty()
                            || page == "about:blank")));
            if block(&uri, resource_type(context), &page, top) {
                refuse(&env, &args)?;
                return Ok(());
            }
            match decide(&uri, document) {
                Verdict::Allow => {}
                Verdict::Deny => refuse(&env, &args)?,
                Verdict::Later(register) => {
                    let held = Held {
                        deferral: args.GetDeferral()?,
                        args: args.clone(),
                        env: env.clone(),
                    };
                    let post = post.clone();
                    register(Box::new(move |allowed| {
                        // This closure runs on the UI thread (`post`), where `held` was made (the SAFETY note
                        // of the enclosing block covers its calls).
                        post(Box::new(move || {
                            let held = held;
                            if !allowed {
                                let _ = refuse(&held.env, &held.args);
                            }
                            let _ = held.deferral.Complete();
                        }));
                    }));
                }
            }
        }
        Ok(())
    }));
    register(controller, &navigation, &handler)
}

fn register(
    controller: &ICoreWebView2Controller,
    navigation: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2NavigationStartingEventHandler,
    handler: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebResourceRequestedEventHandler,
) -> Result<(), String> {
    let fail =
        |what: &str, e: windows::core::Error| format!("WebView2 request guard ({what}): {e}");
    // SAFETY: the controller is Tauri's live WebView2 controller on the UI thread; the filter and the
    // handler registration only take owned or borrowed COM objects; every HRESULT is checked.
    unsafe {
        let core = controller.CoreWebView2().map_err(|e| fail("webview", e))?;
        let mut navigation_token = 0;
        core.add_NavigationStarting(navigation, &raw mut navigation_token)
            .map_err(|e| fail("navigation metadata", e))?;
        let filter = HSTRING::from("*");
        // Since runtime 1.0.2592 frames need the request-source-kinds variant to be seen at all.
        match core.cast::<ICoreWebView2_22>() {
            Ok(core22) => core22
                .AddWebResourceRequestedFilterWithRequestSourceKinds(
                    &filter,
                    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                    COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
                )
                .map_err(|e| fail("filter", e))?,
            Err(_) => core
                .AddWebResourceRequestedFilter(&filter, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)
                .map_err(|e| fail("filter", e))?,
        }
        let mut token = Default::default();
        core.add_WebResourceRequested(handler, &raw mut token)
            .map_err(|e| fail("handler", e))?;
    }
    Ok(())
}

fn resource_type(context: COREWEBVIEW2_WEB_RESOURCE_CONTEXT) -> &'static str {
    match context {
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT => "document",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE => "image",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET => "style-sheet",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT => "script",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT => "font",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA => "media",
        _ => "raw",
    }
}
