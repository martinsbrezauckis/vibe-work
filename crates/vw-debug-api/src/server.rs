//! Starting and stopping the Debug API: enablement, loopback bind on a random port, the per-run
//! descriptor, the guarded router, graceful shutdown, and the core's "Debug API on" flag.
//!
//! Depends on: axum, tokio, vw-core-api (`Core`, `DebugApiFlag`), vw-platform (errors), `config`,
//! `token`, `auth`, `routes`, `registry`. Called by: the app, tests, the private debug host.
//! Invariants:
//! - Disabled means nothing is bound: `start` returns `Ok(None)` before any socket exists.
//! - The listener is `127.0.0.1:0` (IPv4 loopback, OS-chosen port); there is no fallback port
//!   list (ShellX's #311 orphan-socket problem cannot arise with port 0).
//! - The descriptor is written and verified user-only before the first connection is accepted;
//!   if that fails the listener is dropped and `start` fails.
//! - While running, the core's flag says `enabled` with the reason; `shutdown` (or dropping the
//!   handle) turns it off, ends every SSE stream and removes the descriptor.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::middleware;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use vw_core_api::{Core, DebugApiFlag};
use vw_platform::PrivateFileError;

use crate::auth::guard;
use crate::config::{DebugApiConfig, EnabledReason};
use crate::registry::UiRoutes;
use crate::routes::router;
use crate::token::{TokenStore, write_descriptor};

/// Why the Debug API could not start.
#[derive(Debug, thiserror::Error)]
pub enum DebugApiError {
    /// The token needs the person's unlocked vault before the listener can start.
    #[error("Unlock the vault to start the Debug API.")]
    VaultUnavailable,
    /// Vault storage or unlock failed; no plaintext fallback is permitted.
    #[error(transparent)]
    Vault(#[from] vw_keys::KeyError),
    /// Copying or regenerating a vault token belongs to the person in the window.
    #[error(
        "The vault is managed in the window by you; the Debug API has the agent's access here."
    )]
    UserOnly,
    /// Binding the loopback listener failed.
    #[error("cannot listen on 127.0.0.1: {0}")]
    Bind(std::io::Error),
    /// The OS random source failed, so no token could be made.
    #[error("no randomness for the token: {0}")]
    Entropy(String),
    /// The descriptor directory or file could not be made private.
    #[error(transparent)]
    Private(#[from] PrivateFileError),
    /// A stale descriptor could not be replaced.
    #[error("cannot replace descriptor {path}: {source}")]
    Descriptor {
        /// The file.
        path: String,
        /// The OS error.
        source: std::io::Error,
    },
    /// The written descriptor was readable by others; it was removed.
    #[error("descriptor {0} is not user-only; refusing to start")]
    NotPrivate(String),
}

/// What every handler can read.
#[derive(Clone)]
pub(crate) struct ApiState {
    pub(crate) core: Core,
    pub(crate) token: TokenStore,
    pub(crate) port: u16,
    pub(crate) allowed_origins: Arc<[String]>,
    pub(crate) ui_routes: UiRoutes,
    pub(crate) body_limit: usize,
    pub(crate) reason: EnabledReason,
    pub(crate) keep_alive: Duration,
    pub(crate) stop: watch::Receiver<bool>,
    /// The app window; `None` in runs without one.
    pub(crate) window: Option<crate::window_routes::SharedAppWindow>,
}

/// A running Debug API.
pub struct DebugApi {
    core: Core,
    port: u16,
    descriptor: PathBuf,
    token: TokenStore,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl DebugApi {
    /// Starts the Debug API when the config enables it; `Ok(None)` (and nothing bound) when not.
    ///
    /// # Errors
    /// [`DebugApiError`] when binding, the token or the private descriptor fails; nothing is left
    /// listening then.
    pub async fn start(core: Core, config: DebugApiConfig) -> Result<Option<Self>, DebugApiError> {
        if !(cfg!(debug_assertions) && config.test_token_file) {
            crate::token::remove_legacy_token(&config.descriptor_dir)?;
        }
        let Some(reason) = config.enablement() else {
            core.set_debug_api_flag(DebugApiFlag {
                enabled: false,
                reason: "off".to_owned(),
            });
            info!("Debug API is off (no socket opened)");
            return Ok(None);
        };
        let token =
            TokenStore::load_or_create(&config.descriptor_dir, config.keys, config.test_token_file)
                .await?;
        let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .map_err(DebugApiError::Bind)?;
        let port = listener.local_addr().map_err(DebugApiError::Bind)?.port();
        let descriptor = write_descriptor(&config.descriptor_dir, port, token.path())?;
        let (stop, stop_rx) = watch::channel(false);
        let state = ApiState {
            core: core.clone(),
            token: token.clone(),
            port,
            allowed_origins: config.allowed_origins.into(),
            ui_routes: config.ui_routes,
            body_limit: config.body_limit,
            reason,
            keep_alive: config.keep_alive,
            stop: stop_rx.clone(),
            window: config.window,
        };
        // Order: the guard (Host, Origin, token) runs first; every answer it lets through, the routes' and its
        // own, passes the secret check last (one choke point for the secret rule).
        let app = router()
            .layer(DefaultBodyLimit::max(config.body_limit))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::body_limit::structured,
            ))
            .layer(middleware::from_fn_with_state(state.clone(), guard))
            .layer(middleware::from_fn(crate::secret_guard::guard_secrets))
            .with_state(state);
        let mut shutdown = stop_rx;
        let task = tokio::spawn(async move {
            let served = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown.wait_for(|stop| *stop).await;
                })
                .await;
            if let Err(error) = served {
                warn!(%error, "Debug API server ended with an error");
            }
        });
        core.set_debug_api_flag(DebugApiFlag {
            enabled: true,
            reason: reason.as_str().to_owned(),
        });
        info!(
            port,
            descriptor = %descriptor.display(),
            reason = reason.as_str(),
            "Debug API listening on 127.0.0.1 (port in the descriptor file)"
        );
        Ok(Some(Self {
            core,
            port,
            descriptor,
            token,
            stop,
            task: Some(task),
        }))
    }

    /// The bound port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://127.0.0.1:<port>`.
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The nonsecret per-run descriptor file (user-only).
    #[must_use]
    pub fn descriptor_path(&self) -> &Path {
        &self.descriptor
    }

    /// The optional plaintext export for an explicitly selected development fixture.
    /// Always `None` in release builds.
    #[must_use]
    pub fn token_path(&self) -> Option<&Path> {
        self.token.path()
    }

    /// Replaces the vault token, then the in-memory bearer. The old one stops working at
    /// once (the next request that carries it gets 401).
    ///
    /// # Errors
    /// [`DebugApiError`] when the unlocked vault cannot save the replacement; the old token
    /// keeps working then.
    pub async fn rotate_token(&self) -> Result<(), DebugApiError> {
        self.token.rotate().await?;
        info!("Debug API token regenerated");
        Ok(())
    }

    /// Stops accepting, ends open streams, waits up to 5 s for requests in flight, removes the
    /// descriptor and turns the flag off.
    pub async fn shutdown(mut self) {
        self.stop.send_replace(true);
        if let Some(task) = self.task.take() {
            let abort = task.abort_handle();
            if tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .is_err()
            {
                warn!("Debug API did not stop within 5 s; aborting it");
                abort.abort();
            }
        }
        self.finish();
    }

    fn finish(&self) {
        let _ = std::fs::remove_file(&self.descriptor);
        self.core.set_debug_api_flag(DebugApiFlag {
            enabled: false,
            reason: "off".to_owned(),
        });
    }
}

impl Drop for DebugApi {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            self.stop.send_replace(true);
            task.abort();
            self.finish();
        }
    }
}
