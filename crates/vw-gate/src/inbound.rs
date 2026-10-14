//! Classification of server-to-client requests, so the core knows which handler owns each.
//!
//! Depends on: nothing. Called by: the core's server-request handler (via `classify_server_request`).
//! Invariant: every name in `vw_protocol::SERVER_REQUEST_METHODS` maps to a known kind (tested);
//! an unknown name must be answered with an error by the core, never ignored.

/// Which part of the core owns an inbound server request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundKind {
    /// `callback/call`: register it with the gate and surface it to the UI
    /// (`Gate::register_callback`); the answer goes through `Gate::answer_callback`.
    CallbackCall,
    /// `clientTool/*`: runs in our process, outside Vibe's sandbox, so the handler must contain it.
    /// The unified harness does not send these.
    ClientTool,
    /// Not a method this version knows.
    Unknown,
}

/// The owner of the inbound request `method`.
#[must_use]
pub fn classify_server_request(method: &str) -> InboundKind {
    match method {
        "callback/call" => InboundKind::CallbackCall,
        m if m.starts_with("clientTool/") && vw_protocol::SERVER_REQUEST_METHODS.contains(&m) => {
            InboundKind::ClientTool
        }
        _ => InboundKind::Unknown,
    }
}
