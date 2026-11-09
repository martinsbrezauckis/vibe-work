//! Independent chat browser windows using Tauri's shared desktop window API.
//! A single guarded child fills each window; native engine adapters remain the same as panel pages.
use super::TauriEngine;
use std::sync::PoisonError;
use tauri::{
    LogicalPosition, LogicalSize, Manager, Webview, Window, WindowBuilder, WindowEvent, Wry,
};
use vw_preview::{EngineError, PageWindowAction, PageWindowInfo, Placement, Rect};

fn failure(error: impl std::fmt::Display) -> EngineError {
    EngineError::Platform(error.to_string())
}

fn info(window: &Window<Wry>) -> Result<PageWindowInfo, EngineError> {
    let size = window
        .inner_size()
        .map_err(failure)?
        .to_logical::<f64>(window.scale_factor().map_err(failure)?);
    Ok(PageWindowInfo {
        window_id: window.label().into(),
        focused: window.is_focused().map_err(failure)?,
        visible: window.is_visible().map_err(failure)?
            && !window.is_minimized().map_err(failure)?,
        width: size.width,
        height: size.height,
    })
}

impl TauriEngine {
    pub(super) fn mount_window(
        &self,
        spec: &mut vw_preview::MountSpec,
    ) -> Result<(Window<Wry>, bool), EngineError> {
        if let Some(window) = self.page_window_native(&spec.id.session_id) {
            spec.placement = Some(Self::window_placement(&window)?);
            return Ok((window, true));
        }
        self.app
            .get_window(&self.window_label)
            .map(|window| (window, false))
            .ok_or_else(|| failure("The main window is not open."))
    }

    pub(super) fn page_window_native(&self, page: &str) -> Option<Window<Wry>> {
        self.page_windows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(page)
            .cloned()
    }

    pub(super) async fn page_window_inner(
        &self,
        page: String,
        action: PageWindowAction,
    ) -> Result<Option<PageWindowInfo>, EngineError> {
        let current = self.page_window_native(&page);
        if action == PageWindowAction::Open {
            if current.is_some() {
                return Err(failure("This browser page already has a window."));
            }
            let app = self.app.clone();
            // Keep opaque host ids short enough to survive the ordinary key-shaped text scrubber.
            let suffix: String = page
                .strip_prefix("web-page-")
                .unwrap_or(&page)
                .chars()
                .take(16)
                .collect();
            let label = format!("window-{suffix}");
            let window = tokio::task::spawn_blocking(move || {
                WindowBuilder::new(&app, label)
                    .title("Vibe Work — Browser")
                    .inner_size(1100.0, 760.0)
                    .min_inner_size(400.0, 300.0)
                    .focused(false)
                    .build()
            })
            .await
            .map_err(failure)?
            .map_err(failure)?;
            #[cfg(target_os = "linux")]
            {
                let owned = window.clone();
                window
                    .run_on_main_thread(move || {
                        use gtk::prelude::*;
                        if let Ok(native) = owned.gtk_window() {
                            native.set_widget_name(crate::window_shot::BROWSER_WINDOW_WIDGET);
                        }
                    })
                    .map_err(failure)?;
            }
            let hooks = self.hooks()?;
            let owner = page.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    hooks.window_close_requested(&owner);
                }
            });
            let snapshot = info(&window)?;
            self.page_windows
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(page, window);
            return Ok(Some(snapshot));
        }
        let Some(window) = current else {
            return if action == PageWindowAction::Close {
                Ok(None)
            } else {
                Err(failure("This browser window is closed."))
            };
        };
        match action {
            PageWindowAction::Close => {
                window.destroy().map_err(failure)?;
                self.page_windows
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&page);
                Ok(None)
            }
            PageWindowAction::Focus => {
                window.unminimize().map_err(failure)?;
                window.show().map_err(failure)?;
                window.set_focus().map_err(failure)?;
                for _ in 0..20 {
                    let snapshot = info(&window)?;
                    if snapshot.focused {
                        return Ok(Some(snapshot));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                Err(failure(
                    "The operating system did not focus this browser window. Bring it to the front before continuing.",
                ))
            }
            PageWindowAction::Inspect => info(&window).map(Some),
            PageWindowAction::Open => unreachable!(),
        }
    }

    pub(super) fn window_placement(window: &Window<Wry>) -> Result<Placement, EngineError> {
        let snapshot = info(window)?;
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            width: snapshot.width,
            height: snapshot.height,
        };
        bounds.checked().map_err(failure)?;
        Ok(Placement { bounds, zoom: 1.0 })
    }

    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            clippy::unused_async,
            clippy::unused_async_trait_impl,
            reason = "Shared mount flow; GTK waits for its UI-thread placement."
        )
    )]
    pub(super) async fn show_window_page(
        &self,
        view: &Webview<Wry>,
        placement: Placement,
    ) -> Result<(), EngineError> {
        view.set_position(LogicalPosition::new(0.0, 0.0))
            .map_err(failure)?;
        view.set_size(LogicalSize::new(
            placement.bounds.width,
            placement.bounds.height,
        ))
        .map_err(failure)?;
        view.set_zoom(1.0).map_err(failure)?;
        view.set_auto_resize(true).map_err(failure)?;
        #[cfg(target_os = "linux")]
        crate::platform::with_native(
            view,
            "show browser window page",
            std::time::Duration::from_secs(5),
            |native, reply| {
                use gtk::prelude::*;
                let view = native.inner();
                view.set_size_request(1, 1);
                view.set_hexpand(true);
                view.set_vexpand(true);
                view.set_child_visible(true);
                view.show();
                let _ = reply.send(Ok(()));
            },
        )
        .await?;
        view.show().map_err(failure)
    }
}
