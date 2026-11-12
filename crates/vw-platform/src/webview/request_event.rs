//! Portable request observations shared by the macOS delegate and the browser log adapter.
//! Owns bounded metadata only; native pointers and response bodies never cross this boundary.
use std::sync::Arc;

/// What happened to one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestStep {
    /// The request was sent.
    Sent,
    /// A redirect to `url`.
    Redirected,
    /// The response headers arrived (`status`).
    Response,
    /// The load ended (`error` when it failed).
    Completed,
}

/// One watched request event (metadata only, never bodies).
#[derive(Clone, Debug)]
pub struct RequestEvent {
    /// `WebKit`'s id of the load (stable across its steps).
    pub id: u64,
    /// The step.
    pub step: RequestStep,
    /// The URL of this step (the new URL for a redirect).
    pub url: String,
    /// URL of the response (the previous URL when this event redirects).
    pub response_url: Option<String>,
    /// The HTTP method of the request, or its original method when only a response is available.
    pub method: String,
    /// `WebKit`'s resource type number (`_WKResourceLoadInfoResourceType`), -1 when unknown.
    pub resource_type: i64,
    /// The HTTP status of a response or completion, 0 when none.
    pub status: i64,
    /// Served from the cache.
    pub from_cache: bool,
    /// A document load in the main frame, not a subframe navigation.
    pub main_document: bool,
    /// Request header fields, present only when observed with a request.
    pub request_headers: Option<HeaderFields>,
    /// Response header fields, present only for an HTTP response.
    pub response_headers: Option<HeaderFields>,
    /// The error text of a failed load.
    pub error: Option<String>,
}

/// Receives the events (main thread).
pub type RequestSink = Arc<dyn Fn(RequestEvent) + Send + Sync>;

/// Bounded native header fields; browser tools apply access checks and redaction before returning them.
#[derive(Clone, Debug, Default)]
pub struct HeaderFields {
    /// At most 32 fields, names up to 128 and values up to 512 characters.
    pub values: std::collections::BTreeMap<String, String>,
    /// Some fields or characters were omitted.
    pub truncated: bool,
}
