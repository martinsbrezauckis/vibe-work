//! Mistral's work tools for the app's own calls: document **Libraries** (`/v1/libraries`, search
//! through the Conversations API's `document_library` tool) and **image generation** (the Conversations API's
//! `image_generation` tool).
//!
//! Modules: `conversations` (one non-streaming `POST /v1/conversations` with built-in tools, `store: false`, and
//! the parts of its answer we read), `libraries`, `images`. This file holds what they share: the error type, the
//! key-bearing JSON request helper and the multipart body builder.
//!
//! Depends on: reqwest, `serde_json`, `auth`. Called by: vw-keys (`KeyService` work calls).
//! Invariants:
//! - The key goes only into the sensitive, zeroized `Authorization` header (`crate::auth::bearer`), and only to the
//!   API base the caller gives (never to a download host: `images` fetches signed URLs without it).
//! - No error text carries a response body (a 4xx body can echo the request: a prompt, a file name); only a status
//!   and a fixed sentence.
//! - Every answer is read with a size cap; a redirect is never followed (the client's policy).

pub mod conversations;
pub mod images;
pub mod libraries;

use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;

/// Largest JSON answer read from the API (libraries list, a conversation answer).
const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;

/// Why a work call failed. Texts are shown to the user (and to the agent) as they are.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkError {
    /// HTTP 401.
    #[error("Mistral did not accept the API key (401). Check it in Settings.")]
    Unauthorized,
    /// HTTP 402: billing.
    #[error(
        "This key's workspace cannot use this right now (402). Check its plan and billing in Mistral's console."
    )]
    Payment,
    /// HTTP 403: plan, permission or no access to that library.
    #[error(
        "Mistral refused this request (403): the key's plan or permissions, or no access to that library."
    )]
    Forbidden,
    /// HTTP 404.
    #[error("Mistral does not know that item (404): it may have been deleted.")]
    NotFound,
    /// HTTP 413.
    #[error("Mistral says the file is too large (413).")]
    TooLarge,
    /// HTTP 400/422.
    #[error("Mistral could not use this request ({0}).")]
    Rejected(u16),
    /// HTTP 429.
    #[error("Mistral is rate-limiting this key right now (429). Try again in a minute.")]
    RateLimited,
    /// HTTP 5xx.
    #[error("Mistral had a problem ({0}). Try again later.")]
    Server(u16),
    /// Any other status (a redirect included: never followed).
    #[error("Mistral answered with an unexpected status ({0}).")]
    Unexpected(u16),
    /// No answer, or an answer that could not be read.
    #[error("Could not reach Mistral: {0}.")]
    Network(String),
    /// The answer was readable JSON but not the expected shape.
    #[error("Mistral's answer was not in the expected form ({0}).")]
    Shape(&'static str),
    /// The key cannot be put in a header.
    #[error("The stored key contains characters a key cannot have.")]
    MalformedKey,
    /// The image tool ran but made no image (its own reason, sanitized and capped).
    #[error("No image was made: {0}")]
    NoImage(String),
    /// A downloaded file is not what it should be (not an image, too large, a refused address).
    #[error("{0}")]
    BadFile(String),
}

/// Maps a non-2xx status to its error.
#[must_use]
pub fn status_error(status: u16) -> WorkError {
    match status {
        401 => WorkError::Unauthorized,
        402 => WorkError::Payment,
        403 => WorkError::Forbidden,
        404 => WorkError::NotFound,
        413 => WorkError::TooLarge,
        400 | 422 => WorkError::Rejected(status),
        429 => WorkError::RateLimited,
        500..=599 => WorkError::Server(status),
        other => WorkError::Unexpected(other),
    }
}

pub(crate) fn network(error: &reqwest::Error) -> WorkError {
    WorkError::Network(
        if error.is_timeout() {
            "the request timed out"
        } else if error.is_connect() {
            "the connection failed"
        } else {
            "the request failed"
        }
        .to_owned(),
    )
}

/// One API request with the key. `body`: `None` for GET/DELETE, else `(bytes, content type)`.
pub(crate) struct ApiCall<'a> {
    pub method: reqwest::Method,
    pub path: String,
    pub body: Option<(Vec<u8>, String)>,
    pub timeout: Duration,
    pub key: &'a str,
}

/// Sends `call` to `api_base` and returns the status and the body (capped at `max` bytes).
///
/// # Errors
/// [`WorkError::MalformedKey`], [`WorkError::Network`] (also for an answer over `max`).
pub(crate) async fn send(
    client: &reqwest::Client,
    api_base: &str,
    call: ApiCall<'_>,
    max: usize,
) -> Result<(u16, Vec<u8>), WorkError> {
    let auth = crate::auth::bearer(call.key).ok_or(WorkError::MalformedKey)?;
    let url = format!("{}{}", api_base.trim_end_matches('/'), call.path);
    let mut request = client
        .request(call.method, url)
        .header(AUTHORIZATION, auth)
        .timeout(call.timeout);
    if let Some((bytes, content_type)) = call.body {
        request = request.header(CONTENT_TYPE, content_type).body(bytes);
    }
    let response = request.send().await.map_err(|e| network(&e))?;
    let status = response.status().as_u16();
    let bytes = read_capped(response, max).await?;
    Ok((status, bytes))
}

/// Reads a response body, refusing one larger than `max` bytes (checked while streaming, not after).
pub(crate) async fn read_capped(
    mut response: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, WorkError> {
    if response
        .content_length()
        .is_some_and(|n| usize::try_from(n).map_or(true, |n| n > max))
    {
        return Err(WorkError::BadFile(
            "the answer is larger than Vibe Work accepts".to_owned(),
        ));
    }
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| network(&e))? {
        if out.len() + chunk.len() > max {
            return Err(WorkError::BadFile(
                "the answer is larger than Vibe Work accepts".to_owned(),
            ));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// A JSON call: the parsed answer of a 2xx, else the status's error. An empty 2xx body reads as `null`.
pub(crate) async fn json_call(
    client: &reqwest::Client,
    api_base: &str,
    call: ApiCall<'_>,
) -> Result<Value, WorkError> {
    let (status, bytes) = send(client, api_base, call, MAX_JSON_BYTES).await?;
    if !(200..300).contains(&status) {
        return Err(status_error(status));
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| WorkError::Shape("not JSON"))
}

/// The JSON body of a request.
pub(crate) fn json_body(value: &Value) -> (Vec<u8>, String) {
    (
        value.to_string().into_bytes(),
        "application/json".to_owned(),
    )
}

/// A file name safe inside a multipart header: the last path part only, no quotes, backslashes or control
/// characters, at most 200 characters; an empty result becomes `file`.
#[must_use]
pub fn header_file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '"' { '_' } else { c })
        .take(200)
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_owned();
    if trimmed.is_empty() {
        "file".to_owned()
    } else {
        trimmed
    }
}

/// A multipart body with text `fields` and one file part `file` (`name`, MIME type, bytes), and its content
/// type. The boundary is chosen so it does not occur in any part (a counter is added until it does not).
#[must_use]
pub fn multipart(
    fields: &[(&str, &str)],
    file_field: &str,
    file_name: &str,
    mime: &str,
    bytes: &[u8],
) -> (Vec<u8>, String) {
    let mut n: u32 = 0;
    let boundary = loop {
        let candidate = format!("----vibework-part-{n:08x}-7c1e");
        let probe = candidate.as_bytes();
        let clash = bytes.windows(probe.len()).any(|w| w == probe)
            || fields.iter().any(|(_, v)| v.contains(&candidate));
        if !clash {
            break candidate;
        }
        n = n.wrapping_add(1);
    };
    let mut body = Vec::with_capacity(bytes.len() + 512);
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{}\"\r\nContent-Type: {mime}\r\n\r\n",
            header_file_name(file_name)
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (body, format!("multipart/form-data; boundary={boundary}"))
}

/// A short, single-line excerpt of text from Mistral (a tool's error, a model's sentence) for an error text:
/// control characters dropped, at most `max` characters.
#[must_use]
pub fn excerpt(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        let cut: String = flat.chars().take(max).collect();
        format!("{cut}…")
    } else {
        flat
    }
}
