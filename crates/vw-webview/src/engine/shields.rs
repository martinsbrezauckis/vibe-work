//! Native Linux ad-filter updates; compiles before installation and rejects stale completions.
//! Depends on the platform `WebKitGTK` bridge; called before first navigation and by Browser settings.
#[cfg(target_os = "linux")]
impl super::TauriEngine {
    pub(super) async fn apply_filter(
        &self,
        webview: &tauri::Webview<tauri::Wry>,
        label: &str,
        rules: String,
    ) -> Result<(), vw_preview::EngineError> {
        use tauri::Manager as _;
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
                vw_platform::webview::set_content_filter(
                    &native.inner(),
                    &directory,
                    Some(&rules),
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
