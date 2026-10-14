//! The key test call: `GET /v1/models` with the stored key (no prompt is sent, nothing is spent),
//! with clear error texts for the cases the setup screen must explain (401/403/network).
//!
//! Depends on: reqwest, `serde_json`, `client`. Called by: vw-keys (`testKey`).
//! Invariant: neither the key nor the response body is put into an error text (a 4xx body could
//! echo request data); only the status and a fixed explanation.

use std::time::Duration;

use reqwest::header::AUTHORIZATION;

/// Mistral's public API base.
pub const DEFAULT_API_BASE: &str = "https://api.mistral.ai";

/// What a successful test call found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestCallOk {
    /// Number of models the key can use.
    pub model_count: usize,
}

/// Why the test call failed. Texts are shown to the user as is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TestCallError {
    /// HTTP 401.
    #[error("Mistral did not accept this key (401). Check that it is complete and not revoked.")]
    Unauthorized,
    /// HTTP 403.
    #[error(
        "This key is valid but not allowed to use the API (403). Check the workspace's plan and permissions."
    )]
    Forbidden,
    /// HTTP 429.
    #[error("Mistral is rate-limiting this key right now (429). Try again in a minute.")]
    RateLimited,
    /// HTTP 5xx.
    #[error("Mistral's API had a problem ({0}). Try again later.")]
    Server(u16),
    /// Any other status.
    #[error("Mistral answered with an unexpected status ({0}).")]
    Unexpected(u16),
    /// No answer (DNS, TLS, proxy, timeout).
    #[error("Could not reach Mistral: {0}. Check the network or proxy settings.")]
    Network(String),
    /// The key contains characters an HTTP header cannot carry.
    #[error("The key contains characters a key cannot have.")]
    MalformedKey,
}

/// Lists the models the key can use, at `api_base` (normally [`DEFAULT_API_BASE`]).
///
/// # Errors
/// [`TestCallError`] (see its variants).
pub async fn list_models(
    client: &reqwest::Client,
    api_base: &str,
    key: &str,
) -> Result<TestCallOk, TestCallError> {
    let auth = crate::auth::bearer(key).ok_or(TestCallError::MalformedKey)?;
    let url = format!("{}/v1/models", api_base.trim_end_matches('/'));
    let response = client
        .get(url)
        .header(AUTHORIZATION, auth)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| TestCallError::Network(network_reason(&e)))?;
    let status = response.status().as_u16();
    match status {
        200..=299 => {
            let bytes = response
                .bytes()
                .await
                .map_err(|e| TestCallError::Network(network_reason(&e)))?;
            let body: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|_| TestCallError::Network("the answer could not be read".to_owned()))?;
            let model_count = body
                .get("data")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            Ok(TestCallOk { model_count })
        }
        401 => Err(TestCallError::Unauthorized),
        403 => Err(TestCallError::Forbidden),
        429 => Err(TestCallError::RateLimited),
        500..=599 => Err(TestCallError::Server(status)),
        other => Err(TestCallError::Unexpected(other)),
    }
}

/// A short reason without the URL (reqwest's `Display` includes it; the URL is fine but the
/// message is clearer without) and never anything from the request.
fn network_reason(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "the request timed out".to_owned()
    } else if error.is_connect() {
        "the connection failed".to_owned()
    } else if error.is_decode() {
        "the answer could not be read".to_owned()
    } else {
        "the request failed".to_owned()
    }
}
