//! The `WebView2` `DevTools` protocol channel of one webview: method calls on the page's own session or on a
//! flat child session (frames in other processes), and protocol events with their session. Answers come from
//! the browser process, never through page-owned JavaScript.
//!
//! Depends on: `webview2-com` (`ICoreWebView2_11` for child sessions, event arguments 2 for their ids). Called by:
//! vw-webview's frame registry, on the webview's UI thread; every call answers through its callback exactly once.

use webview2_com::{
    CallDevToolsProtocolMethodCompletedHandler, DevToolsProtocolEventReceivedEventHandler,
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_11, ICoreWebView2Controller,
        ICoreWebView2DevToolsProtocolEventReceivedEventArgs2,
    },
    take_pwstr,
};
use windows::core::{HSTRING, Interface, PWSTR};

/// The native controller retained for last-resort session teardown.
pub type Controller = ICoreWebView2Controller;

/// Completes one protocol call with its JSON result, or the protocol's error.
pub type CallDone = Box<dyn FnOnce(Result<String, String>)>;
/// Receives one protocol event: session id (`""` for the page's own), method, JSON parameters.
pub type OnEvent = Box<dyn Fn(&str, &str, &str)>;

/// The protocol channel of one live webview.
#[derive(Clone)]
pub struct DevTools {
    core: ICoreWebView2_11,
}

/// A key for per-webview state: the controller's COM identity, stable while the webview lives.
#[must_use]
pub fn view_key(controller: &ICoreWebView2Controller) -> usize {
    controller.as_raw() as usize
}

/// Tears down the view and its `DevTools` session when native restrictions cannot be cleared.
///
/// # Errors
/// The controller could not be closed. Call only on its UI thread.
pub fn close(controller: &ICoreWebView2Controller) -> Result<(), String> {
    // SAFETY: the caller holds the live controller on the UI thread. Close releases its native sessions.
    unsafe { controller.Close() }.map_err(|e| format!("closing browser controls: {e}"))
}

impl DevTools {
    /// Opens the channel of `controller`'s webview.
    ///
    /// # Errors
    /// A `WebView2` runtime without child-session calls (`ICoreWebView2_11`), or a closed webview.
    pub fn new(controller: &ICoreWebView2Controller) -> Result<Self, String> {
        // SAFETY: Tauri hands over the live controller on its UI thread; both calls only read it.
        let core = unsafe { controller.CoreWebView2() }
            .and_then(|core| core.cast::<ICoreWebView2_11>())
            .map_err(|e| format!("WebView2 DevTools protocol: {e}"))?;
        Ok(Self { core })
    }

    /// Sends `method` with JSON `params` to `session` (`""`: the page's own session).
    pub fn call(&self, session: &str, method: &str, params: &str, done: CallDone) {
        let done = std::rc::Rc::new(std::cell::RefCell::new(Some(done)));
        let answer = done.clone();
        let handler =
            CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, json| {
                if let Some(done) = answer.borrow_mut().take() {
                    done(status.map(|()| json).map_err(|e| e.to_string()));
                }
                Ok(())
            }));
        // SAFETY: UI-thread COM call on the live webview; `WebView2` retains the handler until it answers.
        let started = unsafe {
            self.core.CallDevToolsProtocolMethodForSession(
                &HSTRING::from(session),
                &HSTRING::from(method),
                &HSTRING::from(params),
                &handler,
            )
        };
        if let Err(error) = started
            && let Some(done) = done.borrow_mut().take()
        {
            done(Err(error.to_string()));
        }
    }

    /// Delivers every event named in `events`, from every session, to `on_event` until the webview closes.
    ///
    /// # Errors
    /// The event receiver could not be registered.
    pub fn listen(&self, events: &[&str], on_event: OnEvent) -> Result<(), String> {
        let on_event = std::rc::Rc::new(on_event);
        for name in events {
            let (method, deliver) = ((*name).to_owned(), on_event.clone());
            let handler =
                DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else {
                        return Ok(());
                    };
                    let (mut params, mut session) = (PWSTR::null(), PWSTR::null());
                    // SAFETY: the event arguments are live for this UI-thread callback; strings are freed by
                    // `take_pwstr`. A runtime without session ids reports the page's own session.
                    unsafe {
                        args.ParameterObjectAsJson(&raw mut params)?;
                        if let Ok(with_session) =
                            args.cast::<ICoreWebView2DevToolsProtocolEventReceivedEventArgs2>()
                        {
                            with_session.SessionId(&raw mut session)?;
                        }
                    }
                    let (params, session) = (take_pwstr(params), take_pwstr(session));
                    deliver(&session, &method, &params);
                    Ok(())
                }));
            let mut token = 0;
            // SAFETY: UI-thread COM calls on the live webview; `WebView2` retains the handler.
            unsafe {
                self.core
                    .GetDevToolsProtocolEventReceiver(&HSTRING::from(*name))
                    .and_then(|receiver| {
                        receiver.add_DevToolsProtocolEventReceived(&handler, &raw mut token)
                    })
            }
            .map_err(|e| format!("WebView2 DevTools event {name}: {e}"))?;
        }
        Ok(())
    }
}
