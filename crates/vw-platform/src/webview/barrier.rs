//! The request barrier's types ([`super::guard_requests`] on Windows): the verdict for one request and the
//! callbacks a held request's verdict travels through. Kept apart from `webview.rs` for its size limit.

/// A request barrier's answer (a host-name lookup never blocks the UI thread).
pub enum Verdict {
    /// Send the request.
    Allow,
    /// Answer `403` without sending it.
    Deny,
    /// Decided later: called with the callback that receives the verdict (`true`: send it) from any thread; the
    /// request is held meanwhile (a `WebView2` deferral, completed back on the UI thread through [`PostToUi`]).
    Later(LaterVerdict),
}

/// Receives a held request's verdict (`true`: send it).
pub type VerdictDone = Box<dyn FnOnce(bool) + Send>;

/// Registers the callback a held request's verdict goes to.
pub type LaterVerdict = Box<dyn FnOnce(VerdictDone) + Send>;

/// Decides a request: its URL as text and whether it is a document request (main frame or frame). Called on the UI
/// thread; must return at once.
pub type DecideRequest = Box<dyn Fn(&str, bool) -> Verdict + Send + Sync>;

/// Runs a closure on the UI thread (the app's main thread, which owns the `WebView2` objects).
pub type PostToUi = std::sync::Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;
