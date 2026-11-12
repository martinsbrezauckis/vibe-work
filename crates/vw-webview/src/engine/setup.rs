//! Engine construction, retained mount access and native storage/request-barrier setup.
//! Uses `TauriEngine`'s existing state and platform adapters; called by the engine trait and its child modules.
//! All checks and initial state values retain their original ordering.
use super::{TauriEngine, context_menu};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use tauri::{AppHandle, Manager, Webview, Wry};
use vw_preview::{ConsoleEntry, EngineError, MountId, Placement, PreviewHooks, parse_drain};
#[cfg(windows)]
use {
    crate::platform,
    std::path::PathBuf,
    url::Url,
    vw_preview::{GuardCheck, MountSpec, RequestCheck},
};

impl TauriEngine {
    /// An engine placing previews in `window_label`.
    #[must_use]
    pub fn new(app: AppHandle<Wry>, window_label: &str) -> Self {
        Self {
            app,
            window_label: window_label.to_owned(),
            page_windows: Mutex::default(),
            request_logs: Mutex::default(),
            hooks: OnceLock::new(),
            dialogs: Arc::default(),
            drains: Mutex::new(HashMap::new()),
            initializers: Mutex::new(HashMap::new()),
            context_pollers: context_menu::Pollers::default(),
            context_sink: None,
            shown: Mutex::new(HashMap::new()),
            download_proxies: Mutex::new(HashMap::new()),
            persons: Mutex::new(HashMap::new()),
            visibility: Arc::new(tokio::sync::Mutex::new(())),
            account_anchor: tokio::sync::Mutex::new(None),
            account_operations: tokio::sync::Mutex::new(()),
            audio_lock: Arc::new(tokio::sync::Mutex::new(())),
            foreign_profiles: Mutex::new(HashMap::new()),
            guard: None,
            needs_guard: false,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            filter_generations: Arc::default(),
            #[cfg(windows)]
            filters: Mutex::default(),
            #[cfg(windows)]
            filter_cache: tokio::sync::Mutex::new(None),
            #[cfg(windows)]
            proxy_args_refused: std::sync::atomic::AtomicBool::new(false),
            #[cfg(target_os = "macos")]
            rule_generations: Arc::default(),
        }
    }

    /// The same engine with the agent browser's network guard: mounts use its proxy where this build supports it.
    #[must_use]
    pub fn with_network_guard(mut self, guard: vw_netguard::NetGuard) -> Self {
        self.guard = Some(guard);
        self
    }

    /// Requires the build's protection layers before the first page: a guard listener and proven canary on
    /// Linux/Windows and macOS with `macos-proxy` (the app's default); macOS without it uses content rules only.
    /// Windows needs its own guarded data folder.
    /// A missing or unproven layer refuses the mount with a plain error.
    #[must_use]
    pub fn require_network_guard(mut self) -> Self {
        self.needs_guard = true;
        self
    }

    /// Sends a physical browser context-menu click to the host as a session id and the item only.
    #[must_use]
    pub fn with_context_sink(
        mut self,
        sink: Arc<dyn Fn(String, context_menu::PageMenuChoice) + Send + Sync>,
    ) -> Self {
        self.context_sink = Some(sink);
        self
    }

    /// Records where `label` is now (shown with `placement`, or hidden).
    pub(super) fn note_shown(&self, label: &str, placement: Option<Placement>) {
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = shown.entry(label.to_owned()).or_default();
        entry.visible = placement.is_some();
        if let Some(p) = placement {
            entry.size = Some((p.bounds.width, p.bounds.height));
            entry.placement = Some(p);
        }
    }

    /// Records that `label` is hidden at `placement`'s size (parked).
    pub(super) fn note_parked(&self, label: &str, placement: Placement) {
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = shown.entry(label.to_owned()).or_default();
        entry.visible = false;
        entry.size = Some((placement.bounds.width, placement.bounds.height));
        entry.placement = Some(placement);
    }

    /// Windows: checks the fresh webview's storage and installs the request barrier (`platform::secure`).
    #[cfg(windows)]
    pub(super) async fn secure(
        &self,
        webview: &Webview<Wry>,
        spec: &MountSpec,
        hooks: &Arc<dyn PreviewHooks>,
    ) -> Result<(), EngineError> {
        let label = spec.id.label.clone();
        let hooks = hooks.clone();
        // every request of the page (documents, scripts, images, fetch) under the mount's rules; a host name
        // whose lookup is still running is held with a deferral, never decided by waiting on the UI thread.
        let decide: vw_platform::webview::DecideRequest = Box::new(move |uri, document| {
            let Ok(url) = Url::parse(uri) else {
                return vw_platform::webview::Verdict::Deny;
            };
            match hooks.request_check(&label, &url, document) {
                RequestCheck::Allow => vw_platform::webview::Verdict::Allow,
                RequestCheck::Deny => {
                    // The scheme and port only: a URL can carry a token.
                    tracing::info!(
                        scheme = url.scheme(),
                        port = ?url.port_or_known_default(),
                        document,
                        "preview request refused before it was sent"
                    );
                    vw_platform::webview::Verdict::Deny
                }
                RequestCheck::Later(register) => vw_platform::webview::Verdict::Later(register),
            }
        });
        let app = self.app.clone();
        let post: vw_platform::webview::PostToUi = Arc::new(move |work| {
            // Fails only when the event loop is gone (quitting): the deferral is never completed, so the held
            // request is never sent (fail closed) and goes away with the webview.
            if let Err(error) = app.run_on_main_thread(work) {
                tracing::warn!(%error, "a held preview request could not be answered (closing?): not sent");
            }
        });
        let facts = platform::secure(
            webview,
            spec.storage.key.hex(),
            decide,
            self.block_requests(&spec.id.label)?,
            post,
        )
        .await?;
        // The barrier checks host names at request time (held with a deferral): the navigation hook never waits.
        self.hooks()?.names_checked_later(&spec.id.label);
        self.hooks()?.network_guard(
            &spec.id.label,
            GuardCheck::Layer(vw_preview::GuardLayer::RequestBarrier),
        );
        tracing::info!(
            user_data_folder = %facts.user_data_folder,
            profile = %facts.profile_name,
            "preview storage (WebView2)"
        );
        let profile = PathBuf::from(&facts.profile_path);
        if !profile.starts_with(&spec.storage.dir) {
            tracing::warn!(
                "the WebView2 user data folder is overridden: the preview runs in its own profile there"
            );
            self.foreign_profiles
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(spec.storage.key.hex(), profile);
        }
        Ok(())
    }

    pub(super) fn webview(&self, id: &MountId) -> Result<Webview<Wry>, EngineError> {
        self.app
            .get_webview(&id.label)
            .ok_or(EngineError::NotMounted)
    }

    pub(super) fn hooks(&self) -> Result<Arc<dyn PreviewHooks>, EngineError> {
        self.hooks
            .get()
            .cloned()
            .ok_or_else(|| EngineError::Platform("the preview engine has no hooks yet".to_owned()))
    }
    pub(super) async fn drain_inner(&self, id: &MountId) -> Result<Vec<ConsoleEntry>, EngineError> {
        let script = self
            .drains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id.label)
            .cloned()
            .ok_or(EngineError::NotMounted)?;
        match self.evaluate_inner(id, script).await {
            Ok(json) => Ok(parse_drain(&json)),
            Err(EngineError::Timeout(_)) => {
                Err(EngineError::Timeout("reading the console".to_owned()))
            }
            Err(other) => Err(other),
        }
    }
}
