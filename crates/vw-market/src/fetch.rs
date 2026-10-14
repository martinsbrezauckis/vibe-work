//! Downloads: one trait so tests can serve a local fake index, and the real HTTPS implementation.
//!
//! Depends on: reqwest (through vw-mistral's shared client: OS trust store, redirects never followed), tokio.
//! Called by: `install`, vw-core-api (builds the [`HttpFetcher`]).
//! Invariants:
//! - Only addresses that pass [`crate::catalog::check_url`] are requested (`https://`; loopback `http://` only when
//!   the fetcher was built for a test index).
//! - A redirect is a failure, not followed: catalog addresses are final (GitHub codeload, nodejs.org, the npm
//!   registry), so a redirect means something changed and must be reviewed.
//! - The body is read in chunks and refused as soon as it passes `max` bytes; a 120 s inactivity timeout per chunk.
//! - Errors never contain the address (the catalog-relative name is added by the caller).

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::error::MarketError;

/// Called with the bytes received so far.
pub type Progress<'a> = &'a (dyn Fn(u64) + Send + Sync);

/// A boxed download future.
pub type Fetching<'a> = Pin<Box<dyn Future<Output = Result<Vec<u8>, MarketError>> + Send + 'a>>;

/// Something that downloads bytes.
pub trait Fetcher: Send + Sync {
    /// Downloads `url`, at most `max` bytes, reporting progress.
    fn get<'a>(&'a self, url: &'a str, max: u64, progress: Progress<'a>) -> Fetching<'a>;
}

/// The real downloader.
pub struct HttpFetcher {
    client: reqwest::Client,
    allow_loopback_http: bool,
}

impl HttpFetcher {
    /// A downloader on the shared HTTPS client. `allow_loopback_http` only for a test index.
    ///
    /// # Errors
    /// [`MarketError::Download`] when TLS cannot be set up.
    pub fn new(allow_loopback_http: bool) -> Result<Self, MarketError> {
        let client =
            vw_mistral::https_client().map_err(|e| MarketError::Download(e.to_string()))?;
        Ok(Self {
            client,
            allow_loopback_http,
        })
    }
}

const CHUNK_TIMEOUT: Duration = Duration::from_secs(120);

impl Fetcher for HttpFetcher {
    fn get<'a>(&'a self, url: &'a str, max: u64, progress: Progress<'a>) -> Fetching<'a> {
        Box::pin(async move {
            crate::catalog::check_url(url, self.allow_loopback_http)
                .map_err(|reason| MarketError::Download(reason.to_owned()))?;
            let failed = |e: reqwest::Error| MarketError::Download(e.without_url().to_string());
            let mut response = tokio::time::timeout(CHUNK_TIMEOUT, self.client.get(url).send())
                .await
                .map_err(|_| MarketError::Download("the server did not answer in time".to_owned()))?
                .map_err(failed)?;
            let status = response.status();
            if !status.is_success() {
                return Err(MarketError::Download(format!(
                    "the server answered {}",
                    status.as_u16()
                )));
            }
            if response.content_length().is_some_and(|len| len > max) {
                return Err(MarketError::Download(
                    "larger than its size limit".to_owned(),
                ));
            }
            let mut body = Vec::new();
            loop {
                let chunk = tokio::time::timeout(CHUNK_TIMEOUT, response.chunk())
                    .await
                    .map_err(|_| MarketError::Download("the download stalled".to_owned()))?
                    .map_err(failed)?;
                let Some(chunk) = chunk else { break };
                body.extend_from_slice(&chunk);
                if body.len() as u64 > max {
                    return Err(MarketError::Download(
                        "larger than its size limit".to_owned(),
                    ));
                }
                progress(body.len() as u64);
            }
            Ok(body)
        })
    }
}
