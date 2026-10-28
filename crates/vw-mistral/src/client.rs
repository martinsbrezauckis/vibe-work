//! The shared HTTPS client: on Linux rustls with the `ring` provider (installed once per process)
//! and the OS trust store; on Windows and macOS the OS TLS stack (native-tls). The proxy and certificate settings of
//! `network` (the ones saved in Settings > Network) are applied to every client built here; with none set, the system
//! proxy settings are honoured as before. A 10 s connect timeout and no overall timeout (model streams can run for
//! minutes; callers add their own). **Neither client follows redirects**: every caller sends
//! the key, and a 3xx must never carry it, or the request, to another place.
//!
//! Depends on: reqwest, rustls, `network`. Called by: vw-keys (key test call, OCR, speech), vw-keyproxy.

use std::time::Duration;

/// The client could not be built (TLS setup failed).
#[derive(Debug, thiserror::Error)]
#[error("could not set up HTTPS: {0}")]
pub struct ClientError(pub String);

/// A new HTTPS client for the core's own key-bearing calls (key test, OCR, speech). Cheap to clone; keep
/// one per component. It never follows a redirect: a 3xx comes back as a status, which the callers turn
/// into an "unexpected status" error (before reqwest followed and only dropped the key on
/// a cross-host hop).
///
/// # Errors
/// [`ClientError`] when TLS cannot be set up (no usable trust store).
pub fn https_client() -> Result<reqwest::Client, ClientError> {
    build(reqwest::redirect::Policy::none())
}

/// The model proxy's client: the same settings as [`https_client`] (it never follows a redirect). A 3xx
/// goes back to Vibe unchanged, so the real key is only ever sent to the host the proxy chose (reqwest
/// would drop `Authorization` on a cross-host hop, but the proxy must not rely on that).
///
/// # Errors
/// [`ClientError`] when TLS cannot be set up.
pub fn proxy_client() -> Result<reqwest::Client, ClientError> {
    build(reqwest::redirect::Policy::none())
}

/// A key-bearing client that is built again when the network settings change in a run (they are applied after a
/// vault unlock when the start could not check them). Clones share the slot.
#[derive(Clone)]
pub struct ClientSlot(std::sync::Arc<std::sync::RwLock<reqwest::Client>>);

impl ClientSlot {
    /// A slot with a [`proxy_client`].
    ///
    /// # Errors
    /// [`ClientError`] when TLS cannot be set up.
    pub fn new() -> Result<Self, ClientError> {
        Ok(Self(std::sync::Arc::new(std::sync::RwLock::new(
            proxy_client()?,
        ))))
    }

    /// The client now (requests already sent keep the one they started with).
    #[must_use]
    pub fn get(&self) -> reqwest::Client {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Builds the client again with the current network settings; on an error the old client stays.
    ///
    /// # Errors
    /// [`ClientError`] when TLS cannot be set up.
    pub fn rebuild(&self) -> Result<(), ClientError> {
        let client = proxy_client()?;
        *self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = client;
        Ok(())
    }
}

/// A client builder with this crate's TLS setup and nothing else (the capture's media fetcher adds its
/// own resolver, redirect and proxy rules).
pub fn tls_builder() -> reqwest::ClientBuilder {
    #[cfg(target_os = "linux")]
    {
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::builder().tls_backend_rustls()
    }
    #[cfg(not(target_os = "linux"))]
    {
        reqwest::Client::builder().tls_backend_native()
    }
}

fn build(redirects: reqwest::redirect::Policy) -> Result<reqwest::Client, ClientError> {
    #[cfg(target_os = "linux")]
    let builder = {
        // Installing fails only when a provider is already installed (by us earlier or another
        // crate); either way a default exists afterwards, which is all reqwest needs.
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::builder().tls_backend_rustls()
    };
    #[cfg(not(target_os = "linux"))]
    let builder = reqwest::Client::builder().tls_backend_native();
    // The saved proxy and certificates (Settings > Network); read when the client is built, so a client keeps the
    // settings of the moment it was made.
    crate::network::current()
        .apply(builder)
        .redirect(redirects)
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| ClientError(e.to_string()))
}
