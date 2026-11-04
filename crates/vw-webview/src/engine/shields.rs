//! Native ad-filter updates; compiles before installation and rejects stale completions.
//! Depends on the platform content-rule bridges; called before first navigation and by Browser settings.
#[cfg(any(target_os = "linux", target_os = "macos"))]
impl super::TauriEngine {
    pub(super) async fn apply_filter(
        &self,
        webview: &tauri::Webview<tauri::Wry>,
        label: &str,
        rules: String,
    ) -> Result<(), vw_preview::EngineError> {
        #[cfg(target_os = "linux")]
        use tauri::Manager as _;
        #[cfg(target_os = "linux")]
        let directory = self
            .app
            .path()
            .app_cache_dir()
            .map_err(|error| vw_preview::EngineError::Platform(error.to_string()))?
            .join("browser-filters");
        let generation = self.filter_generations.begin(label);
        let (generations, key) = (self.filter_generations.clone(), label.to_owned());
        let result = crate::platform::with_native(
            webview,
            "applying the ad filter",
            std::time::Duration::from_secs(30),
            move |native, reply| {
                #[cfg(target_os = "linux")]
                vw_platform::webview::set_content_filter(
                    &native.inner(),
                    &directory,
                    Some(&rules),
                    Box::new(move || generations.is_latest(&key, generation)),
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
                #[cfg(target_os = "macos")]
                vw_platform::content_rules::apply_content_rules(
                    native.inner(),
                    &format!("{key}-shields"),
                    &rules,
                    Box::new(move || generations.is_latest(&key, generation)),
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
            },
        )
        .await;
        if result.is_err() {
            self.filter_generations.forget(label);
        }
        result
    }
}

#[cfg(windows)]
pub(super) type FilterSlot =
    std::sync::Arc<std::sync::RwLock<std::sync::Arc<vw_preview::shields::matcher::Matcher>>>;

#[cfg(windows)]
pub(super) type FilterCache = tokio::sync::Mutex<
    Option<(
        String,
        std::sync::Arc<vw_preview::shields::matcher::Matcher>,
    )>,
>;

#[cfg(windows)]
impl super::TauriEngine {
    pub(super) async fn apply_filter(
        &self,
        webview: &tauri::Webview<tauri::Wry>,
        label: &str,
        rules: String,
    ) -> Result<(), vw_preview::EngineError> {
        use std::sync::{Arc, PoisonError, RwLock};
        use vw_preview::{EngineError, shields::matcher::Matcher};
        // Share one compiled index across tabs; parsing and regex construction never run on the UI thread.
        let matcher = {
            let mut cache = self.filter_cache.lock().await;
            if let Some((source, matcher)) = &*cache
                && source == &rules
            {
                matcher.clone()
            } else {
                let source = rules.clone();
                let matcher = tokio::task::spawn_blocking(move || Matcher::new(&rules))
                    .await
                    .map_err(|e| EngineError::Platform(e.to_string()))?
                    .map_err(EngineError::Platform)?;
                let matcher = Arc::new(matcher);
                *cache = Some((source, matcher.clone()));
                matcher
            }
        };
        let for_css = matcher.clone();
        crate::platform::with_native(
            webview,
            "applying the ad filter",
            std::time::Duration::from_secs(30),
            move |native, reply| {
                crate::frames_win::update_cosmetics(
                    &native,
                    for_css,
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
            },
        )
        .await?;
        let mut filters = self.filters.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = filters.get(label) {
            *slot.write().unwrap_or_else(PoisonError::into_inner) = matcher;
        } else {
            filters.insert(label.into(), Arc::new(RwLock::new(matcher)));
        }
        Ok(())
    }

    pub(super) fn block_requests(
        &self,
        label: &str,
    ) -> Result<vw_platform::webview::BlockRequest, vw_preview::EngineError> {
        use std::sync::PoisonError;
        let slot = self
            .filters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(label)
            .cloned()
            .ok_or_else(|| {
                vw_preview::EngineError::Platform("Browser ad filter was not prepared".into())
            })?;
        Ok(Box::new(move |url, kind, page, top| {
            let matcher = slot.read().unwrap_or_else(PoisonError::into_inner).clone();
            let blocked = matcher.blocks(url, kind, page, top);
            if blocked {
                tracing::debug!("browser ad filter blocked a request");
            }
            blocked
        }))
    }
}
