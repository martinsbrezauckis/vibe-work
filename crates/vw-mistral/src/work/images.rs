//! Image generation and editing through the Conversations API's `image_generation` tool.
//!
//! Measured on (docs.mistral.ai `studio/agents/agent-tools/
//! image_generation` still shows `tool_file` chunks, which our `store: false` calls did not get):
//! - the tool runs `generate_image {prompt}` or `edit_image {image_url, prompt}`; its `info.result` is
//!   `{"url": "<signed https URL on *.blob.core.windows.net, valid 1 hour>"}` (a JPEG, `1024x768` in the probe), or an
//!   error sentence (`Error executing tool edit_image: Image not found at URL:...`);
//! - **editing needs an image URL Mistral can fetch**: an inline `data:` image in the input is not passed to the
//!   tool (probe: the model invented `https://example.com/image.png`). So an edit uploads the source image to
//!   `/v1/files` (purpose `image_generation`), asks for a signed URL (`/v1/files/{id}/url?expiry=1`), names it in the
//!   prompt, and deletes the upload afterwards (always, also after a failure);
//! - every run counts as one image call in `usage.connectors.image_generation`, a failed edit too;
//! - limits come back as headers (`x-ratelimit-limit-image-generation-minute: 15`, `-day: 12000` for the probed key).
//!
//! Depends on: `super` (request helpers, multipart, errors), `conversations`. Called by: vw-keys.
//! Invariants:
//! - The download goes only to an address the [`FetchPolicy`] allows (https and a Mistral storage host by default),
//!   **without** the key, without redirects, capped at [`MAX_IMAGE_BYTES`], and the bytes must be a PNG, JPEG or
//!   WebP by their own signature (the extension we save under comes from that, never from the server).
//! - The prompt and the tool's texts are data; error texts quote a tool sentence only sanitized and capped.

use std::time::Duration;

use serde_json::{Value, json};

use super::conversations::{self, Outcome};
use super::{ApiCall, WorkError, excerpt, json_call, multipart, read_capped};

/// Largest image accepted from the download (25 MB; the probe's JPEG was 0.1-0.7 MB).
pub const MAX_IMAGE_BYTES: usize = 25 * 1024 * 1024;
/// Largest source image sent for an edit (10 MB).
pub const MAX_SOURCE_BYTES: usize = 10 * 1024 * 1024;
/// Longest prompt accepted (characters).
pub const MAX_PROMPT_CHARS: usize = 4_000;

/// The instructions of the image conversation (fixed text; the user's request is the input).
pub const IMAGE_INSTRUCTIONS: &str = "You make images with the image generation tool. Call it exactly once for the \
request: generate a new image, or edit the image at the URL given in the request. Do not ask questions; when a detail \
is missing, choose a sensible one.";

/// Where a generated image may be downloaded from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPolicy {
    /// Only `https` addresses (tests set `false` to use a loopback fake).
    pub https_only: bool,
    /// Allowed host names: an exact name, or a suffix starting with `.` (`.blob.core.windows.net`).
    pub hosts: Vec<String>,
}

impl Default for FetchPolicy {
    /// Mistral's image storage as measured: `mistralaiblackforestprod.blob.core.windows.net` (any
    /// `*.blob.core.windows.net` host, so a renamed storage account still works), https only.
    fn default() -> Self {
        Self {
            https_only: true,
            hosts: vec![".blob.core.windows.net".to_owned()],
        }
    }
}

impl FetchPolicy {
    /// True when `url` may be fetched.
    #[must_use]
    pub fn allows(&self, url: &reqwest::Url) -> bool {
        let scheme_ok = url.scheme() == "https" || (!self.https_only && url.scheme() == "http");
        let Some(host) = url.host_str() else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        scheme_ok
            && url.username().is_empty()
            && url.password().is_none()
            && self.hosts.iter().any(|allowed| {
                let allowed = allowed.to_ascii_lowercase();
                if allowed.starts_with('.') {
                    host.ends_with(&allowed) && host.len() > allowed.len()
                } else {
                    host == allowed
                }
            })
    }
}

/// An image type the app saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
    /// WebP.
    Webp,
}

impl ImageKind {
    /// File extension.
    #[must_use]
    pub fn ext(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    /// MIME type.
    #[must_use]
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }
}

/// The image type by the bytes' own signature, `None` for anything else.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Option<ImageKind> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageKind::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageKind::Jpeg)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageKind::Webp)
    } else {
        None
    }
}

/// An image to edit.
pub struct SourceImage<'a> {
    /// Its file name (for the upload).
    pub name: &'a str,
    /// Its bytes (a PNG, JPEG or WebP by signature).
    pub bytes: &'a [u8],
}

/// A made image.
#[derive(Debug, Clone, PartialEq)]
pub struct Made {
    /// The image file.
    pub bytes: Vec<u8>,
    /// Its type (by signature).
    pub kind: ImageKind,
    /// `generate_image` or `edit_image`.
    pub function: String,
    /// The prompt the tool was given (the model may rephrase the request).
    pub prompt: String,
    /// The model's short text about it.
    pub text: String,
    /// Mistral's usage object.
    pub usage: Value,
}

/// Time limits of one image request.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// The source upload and its signed URL (edits only).
    pub upload: Duration,
    /// The conversation (the image is made inside it; 4-14 s measured).
    pub conversation: Duration,
    /// The download of the image.
    pub download: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            upload: Duration::from_secs(10),
            conversation: Duration::from_secs(35),
            download: Duration::from_secs(15),
        }
    }
}

/// Uploads the source image and returns `(file id, signed URL)`.
async fn upload_source(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    source: &SourceImage<'_>,
    timeout: Duration,
) -> Result<(String, String), WorkError> {
    let kind = sniff(source.bytes).ok_or_else(|| {
        WorkError::BadFile("the image to edit is not a PNG, JPEG or WebP".to_owned())
    })?;
    if source.bytes.len() > MAX_SOURCE_BYTES {
        return Err(WorkError::BadFile(
            "the image to edit is larger than 10 MB".to_owned(),
        ));
    }
    let (body, content_type) = multipart(
        &[("purpose", "image_generation")],
        "file",
        source.name,
        kind.mime(),
        source.bytes,
    );
    let answer = json_call(
        client,
        base,
        ApiCall {
            method: reqwest::Method::POST,
            path: "/v1/files".to_owned(),
            body: Some((body, content_type)),
            timeout,
            key,
        },
    )
    .await?;
    let id = answer
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .ok_or(WorkError::Shape("no file id"))?
        .to_owned();
    let signed = json_call(
        client,
        base,
        ApiCall {
            method: reqwest::Method::GET,
            path: format!("/v1/files/{id}/url?expiry=1"),
            body: None,
            timeout,
            key,
        },
    )
    .await;
    if let Some(url) = signed
        .ok()
        .and_then(|v| v.get("url").and_then(Value::as_str).map(str::to_owned))
    {
        Ok((id, url))
    } else {
        delete_file(client, base, key, &id).await;
        Err(WorkError::Shape("no signed URL for the uploaded image"))
    }
}

/// Deletes an uploaded file (best effort; a failure is logged by the caller's absence of a file, not retried).
async fn delete_file(client: &reqwest::Client, base: &str, key: &str, id: &str) {
    let _ = json_call(
        client,
        base,
        ApiCall {
            method: reqwest::Method::DELETE,
            path: format!("/v1/files/{id}"),
            body: None,
            timeout: Duration::from_secs(10),
            key,
        },
    )
    .await;
}

/// The image URL the tool returned, or why there is none.
///
/// # Errors
/// [`WorkError::NoImage`] with the tool's (or the model's) sentence, sanitized and capped.
pub fn image_url(outcome: &Outcome) -> Result<(String, String, String), WorkError> {
    let Some(run) = outcome
        .runs
        .iter()
        .rev()
        .find(|r| r.name == "image_generation")
    else {
        let said = excerpt(&outcome.text, 300);
        return Err(WorkError::NoImage(if said.is_empty() {
            "the model did not use the image tool".to_owned()
        } else {
            format!("the model did not use the image tool; it said: \"{said}\"")
        }));
    };
    let prompt = run
        .arguments
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    match serde_json::from_str::<Value>(&run.result)
        .ok()
        .and_then(|v| v.get("url").and_then(Value::as_str).map(str::to_owned))
    {
        Some(url) => Ok((url, run.function.clone(), prompt)),
        None => Err(WorkError::NoImage(format!(
            "the image tool said: \"{}\"",
            excerpt(&run.result, 300)
        ))),
    }
}

/// Downloads a made image under `policy` (no key, no redirect, capped, checked by signature).
///
/// # Errors
/// [`WorkError::BadFile`] for a refused address, a wrong type or size; [`WorkError`] for transport failures.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    policy: &FetchPolicy,
    timeout: Duration,
) -> Result<(Vec<u8>, ImageKind), WorkError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| WorkError::BadFile("the image address could not be read".to_owned()))?;
    if !policy.allows(&parsed) {
        return Err(WorkError::BadFile(
            "the image address is not one of Mistral's storage hosts; it was not downloaded"
                .to_owned(),
        ));
    }
    let response = client
        .get(parsed)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| super::network(&e))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(WorkError::BadFile(format!(
            "the image download failed ({status})"
        )));
    }
    let bytes = read_capped(response, MAX_IMAGE_BYTES).await?;
    let kind = sniff(&bytes).ok_or_else(|| {
        WorkError::BadFile("the downloaded file is not a PNG, JPEG or WebP image".to_owned())
    })?;
    Ok((bytes, kind))
}

/// One image request.
pub struct ImageJob<'a> {
    /// The model that drives the tool.
    pub model: &'a str,
    /// What to make, or how to change `source`.
    pub prompt: &'a str,
    /// The image to edit (`None`: a new image).
    pub source: Option<SourceImage<'a>>,
    /// Where the result may be downloaded from.
    pub policy: &'a FetchPolicy,
    /// Time limits.
    pub timeouts: Timeouts,
}

/// Makes one image: a new one from the job's prompt, or an edit of its source.
///
/// # Errors
/// [`WorkError`]; a prompt that is empty or over [`MAX_PROMPT_CHARS`] is [`WorkError::BadFile`] (not sent).
pub async fn make(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    job: &ImageJob<'_>,
) -> Result<Made, WorkError> {
    let (model, policy, timeouts) = (job.model, job.policy, job.timeouts);
    let prompt = job.prompt.trim();
    if prompt.is_empty() || prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(WorkError::BadFile(format!(
            "the prompt must be 1 to {MAX_PROMPT_CHARS} characters"
        )));
    }
    let uploaded = match &job.source {
        Some(source) => Some(upload_source(client, base, key, source, timeouts.upload).await?),
        None => None,
    };
    let input = match &uploaded {
        Some((_, url)) => format!("Edit the image at this URL: {url}\n\nThe edit: {prompt}"),
        None => prompt.to_owned(),
    };
    let body = conversations::request_body(
        model,
        IMAGE_INSTRUCTIONS,
        &Value::String(input),
        &json!([{"type": "image_generation"}]),
    );
    let outcome = conversations::run(client, base, key, &body, timeouts.conversation).await;
    if let Some((id, _)) = &uploaded {
        delete_file(client, base, key, id).await;
    }
    let outcome = outcome?;
    let (url, function, used) = image_url(&outcome)?;
    let (bytes, kind) = download(client, &url, policy, timeouts.download).await?;
    Ok(Made {
        bytes,
        kind,
        function,
        prompt: excerpt(&used, 1_000),
        text: excerpt(&outcome.text, 600),
        usage: outcome.usage,
    })
}
