//! Mistral OCR (`POST /v1/ocr`): one document in, its pages as Markdown out.
//!
//! Choice and source (docs):
//! model `mistral-ocr-latest` (the alias follows Mistral's live OCR model; dated ids were retired one
//! day after deprecation), the file inline as a base64 `data:` URL (`document_url` for documents,
//! `image_url` for images; nothing is stored on Mistral's side, unlike `/v1/files` which keeps uploads
//! 30 days), `include_image_base64: false` (only the text is used), `table_format` unset (tables stay
//! inline in the page Markdown).
//!
//! Depends on: reqwest, base64, `serde_json`. Called by: vw-keys (`KeyService::ocr`).
//! Invariants: the key goes only into a sensitive, zeroized `Authorization` header (`auth`); a redirect is
//! never followed (the client's policy); no error text contains the
//! key, the request or the response body (a 4xx body could echo the request); only a status and a fixed
//! sentence. Pages are returned in Mistral's `index` order.

use std::time::Duration;

use base64::Engine as _;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

/// The OCR model alias the app uses.
pub const OCR_MODEL: &str = "mistral-ocr-latest";

/// How long one OCR request may take (a large PDF takes a while; Mistral's own SDK default is longer).
pub const OCR_TIMEOUT: Duration = Duration::from_secs(300);

/// One document to read.
pub struct OcrInput<'a> {
    /// The file's bytes.
    pub bytes: &'a [u8],
    /// Its MIME type (`application/pdf`, `image/png`,...).
    pub mime: &'a str,
    /// True for images (sent as `image_url`).
    pub image: bool,
}

/// What OCR returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrOutput {
    /// The model that read it (Mistral's answer, e.g. `mistral-ocr-2512`).
    pub model: String,
    /// Page Markdown, in page order.
    pub pages: Vec<String>,
    /// `usage_info.pages_processed`, when given.
    pub pages_processed: Option<u32>,
}

/// Why OCR failed. Texts are shown to the user as they are.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OcrError {
    /// HTTP 401.
    #[error("Mistral did not accept the API key (401). Check it in Settings.")]
    Unauthorized,
    /// HTTP 402/403.
    #[error("This key may not use OCR ({0}). Check the workspace's plan and permissions.")]
    Forbidden(u16),
    /// HTTP 413.
    #[error("Mistral says the document is too large (413).")]
    TooLarge,
    /// HTTP 422 / 400: Mistral could not read this file.
    #[error(
        "Mistral could not read this file ({0}). It may be damaged, protected or of a kind OCR does not take."
    )]
    Rejected(u16),
    /// HTTP 429.
    #[error("Mistral is rate-limiting this key right now (429). Try again in a minute.")]
    RateLimited,
    /// HTTP 5xx.
    #[error("Mistral's OCR had a problem ({0}). Try again later.")]
    Server(u16),
    /// Any other status.
    #[error("Mistral answered with an unexpected status ({0}).")]
    Unexpected(u16),
    /// No answer, or an answer that could not be read.
    #[error("Could not reach Mistral OCR: {0}.")]
    Network(String),
    /// The key cannot be put in a header.
    #[error("The stored key contains characters a key cannot have.")]
    MalformedKey,
}

/// The request body (no key in it).
#[must_use]
pub fn request_body(input: &OcrInput<'_>) -> Value {
    let data = base64::engine::general_purpose::STANDARD.encode(input.bytes);
    let url = format!("data:{};base64,{data}", input.mime);
    let document = if input.image {
        json!({"type": "image_url", "image_url": url})
    } else {
        json!({"type": "document_url", "document_url": url})
    };
    json!({"model": OCR_MODEL, "document": document, "include_image_base64": false})
}

/// Reads the pages out of an OCR answer.
fn parse(body: &Value) -> Option<OcrOutput> {
    let mut pages: Vec<(u64, String)> = body
        .get("pages")?
        .as_array()?
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let index = p.get("index").and_then(Value::as_u64).unwrap_or(i as u64);
            let text = p
                .get("markdown")
                .and_then(Value::as_str)
                .unwrap_or_default();
            (index, text.to_owned())
        })
        .collect();
    pages.sort_by_key(|(i, _)| *i);
    Some(OcrOutput {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(OCR_MODEL)
            .to_owned(),
        pages: pages.into_iter().map(|(_, t)| t).collect(),
        pages_processed: body
            .pointer("/usage_info/pages_processed")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
    })
}

fn status_error(status: u16) -> OcrError {
    match status {
        401 => OcrError::Unauthorized,
        402 | 403 => OcrError::Forbidden(status),
        413 => OcrError::TooLarge,
        400 | 422 => OcrError::Rejected(status),
        429 => OcrError::RateLimited,
        500..=599 => OcrError::Server(status),
        other => OcrError::Unexpected(other),
    }
}

fn network(error: &reqwest::Error) -> OcrError {
    OcrError::Network(
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

/// Sends one document to OCR at `api_base` (normally `https://api.mistral.ai`) with `key`, waiting at
/// most `timeout` ([`OCR_TIMEOUT`] in the app).
///
/// # Errors
/// [`OcrError`] (see its variants).
pub async fn ocr(
    client: &reqwest::Client,
    api_base: &str,
    key: &str,
    input: &OcrInput<'_>,
    timeout: Duration,
) -> Result<OcrOutput, OcrError> {
    let auth = crate::auth::bearer(key).ok_or(OcrError::MalformedKey)?;
    let body = request_body(input).to_string();
    let response = client
        .post(format!("{}/v1/ocr", api_base.trim_end_matches('/')))
        .header(AUTHORIZATION, auth)
        .header(CONTENT_TYPE, "application/json")
        .body(body)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| network(&e))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(status_error(status));
    }
    let bytes = response.bytes().await.map_err(|e| network(&e))?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|v| parse(&v))
        .ok_or_else(|| OcrError::Network("the answer could not be read".to_owned()))
}
