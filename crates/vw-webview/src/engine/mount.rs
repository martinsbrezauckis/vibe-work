//! Native child creation and setup before a page is admitted. Called by the engine mount seam.
//! Shares profile, proxy, frame and visibility setup for all three native engines.
use super::{TauriEngine, folder_guard, not_checked, page_origin_for_mount, platform, win_guard};
use std::sync::PoisonError;
use tauri::{LogicalPosition, LogicalSize};
use vw_preview::{EngineError, MountSpec};
impl TauriEngine {
    pub(super) async fn mount_inner(&self, spec: MountSpec) -> Result<(), EngineError> {
        // Windows, Linux: the local-network page the user opened runs on a data folder of its own.
        let mut spec = folder_guard::with_own_folder(spec);
        let hooks = self.hooks()?;
        let mut proxy = self.proxy_for(&spec)?;
        let (window, own_window) = self.mount_window(&mut spec)?;
        let parked = platform::parked_or_smallest(spec.parked);
        // a hidden mount is created at its parked size, outside the window (Windows/macOS: the renderer
        // starts at the viewport the page will lay out at; no frame of it can land on the UI before it is hidden).
        // Linux: our runtime creates children hidden in Tauri's box. Their first allocation is in the preview
        // layer, at the final viewport, so creation cannot divide the main UI's allocation or paint a blank child.
        let shape = spec.placement.unwrap_or(parked);

        // The UI's rectangles are the app page's CSS pixels; the child is positioned in the content view's.
        let in_window = if own_window {
            shape.bounds
        } else {
            page_origin_for_mount(&window, shape.bounds).await?
        };
        let (x, y) = if spec.placement.is_some() {
            (in_window.x, in_window.y)
        } else {
            platform::outside_at(in_window)
        };
        let (position, size) = (
            LogicalPosition::new(x, y),
            LogicalSize::new(shape.bounds.width, shape.bounds.height),
        );
        let webview = match self
            .create(&window, &spec, &hooks, proxy.as_deref(), position, size)
            .await
        {
            Ok(webview) => webview,
            // Windows: an environment with other arguments already uses the folder (one folder for every window,
            // set in a way the settings check did not see): no preview can have the guard's arguments now. The
            // previews' instance refuses this and every later preview (fail closed); another instance creates it
            // with the default arguments and the barrier.
            Err(error)
                if cfg!(windows) && proxy.is_some() && win_guard::other_arguments(&error) =>
            {
                #[cfg(windows)]
                self.proxy_args_refused
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if self.needs_guard {
                    tracing::warn!(%error, "WebView2 refused the guard's browser arguments: previews are off");
                    return Err(win_guard::previews_off(win_guard::OneFolder::Refused));
                }
                tracing::warn!(%error, "preview created with the default browser arguments (request barrier)");
                proxy = None;
                self.create(&window, &spec, &hooks, None, position, size)
                    .await?
            }
            Err(error) => return Err(error),
        };
        let trusted = self.remember_initializer(&spec);
        self.remember_download_proxy(&spec.id.label, proxy.as_deref());
        let setup = async {
            self.native_watches(&webview, &spec.id.label).await?;
            self.apply_filter(
                &webview,
                &spec.id.label,
                hooks.content_filter().unwrap_or_else(|| "[]".into()),
            )
            .await?;
            #[cfg(windows)]
            self.secure(&webview, &spec, &hooks).await?;
            self.place_mounted(&webview, &spec, own_window, parked)
                .await?;
            #[cfg(target_os = "macos")]
            self.macos_layers(&webview, &spec, &hooks, proxy.is_some())
                .await?;
            // Every guarded web mount (a browser/preview mount: URL rules, or the guard's proxy) starts without the
            // service workers earlier pages registered in its storage, on every platform.
            platform::clear_service_workers_at_mount(
                &webview,
                proxy.is_some() || spec.rules.is_some(),
            )
            .await?;
            if proxy.is_some() {
                // The proxy resolves and checks every host at request time: the navigation hook never waits for a
                // lookup on the UI thread for this mount.
                hooks.names_checked_later(&spec.id.label);
            }
            let proven = self
                // macOS: the content rules and proxy route have been installed.
                .check_guard(&webview, &spec.id, proxy.as_deref(), &hooks)
                .await;
            if (self.proxy_required() || cfg!(target_os = "macos") && proxy.is_some()) && !proven {
                return Err(not_checked("its network check could not be confirmed"));
            }
            webview
                .navigate(spec.url.clone())
                .map_err(|e| EngineError::Platform(e.to_string()))
        }
        .await;
        if let Err(error) = setup {
            let _ = self.close_inner(&spec.id);
            return Err(error);
        }
        match spec.placement {
            Some(_) => self.note_shown(&spec.id.label, spec.placement),
            None => self.note_parked(&spec.id.label, parked),
        }
        self.drains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(spec.id.label.clone(), spec.console_drain);
        if spec.context_menu
            && let Some(sink) = &self.context_sink
        {
            self.context_pollers
                .start(webview, &spec.id, sink.clone(), trusted);
        }
        Ok(())
    }
    async fn place_mounted(
        &self,
        webview: &tauri::Webview<tauri::Wry>,
        spec: &MountSpec,
        own_window: bool,
        parked: vw_preview::Placement,
    ) -> Result<(), EngineError> {
        match (own_window, spec.placement) {
            (true, Some(placement)) => self.show_window_page(webview, placement).await,
            (_, Some(placement)) => platform::place(webview, Some(placement)).await,
            (_, None) => platform::park(webview, parked, false).await,
        }
    }
}
