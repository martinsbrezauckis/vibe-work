//! The Windows child page's full-screen placement, separate from the app window's full-screen state.
//! Depends on preview geometry and Tauri; called by native full-screen events and ordinary placement.

use vw_preview::{Placement, Rect};

#[derive(Default)]
struct Layout {
    entered: bool,
    panel: Option<Placement>,
}

impl Layout {
    fn place(&mut self, panel: Placement, window: (f64, f64)) -> Placement {
        self.panel = Some(panel);
        if self.entered {
            Placement {
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: window.0,
                    height: window.1,
                },
                ..panel
            }
        } else {
            panel
        }
    }
}

#[cfg(windows)]
pub(crate) use native::{changed, forget, placement};

#[cfg(windows)]
mod native {
    use super::{Layout, Placement};
    use std::{
        collections::HashMap,
        sync::{LazyLock, Mutex, PoisonError},
    };
    use tauri::{Webview, Wry};
    use vw_preview::EngineError;

    static VIEWS: LazyLock<Mutex<HashMap<String, Layout>>> = LazyLock::new(Mutex::default);

    pub(crate) fn placement(
        view: &Webview<Wry>,
        panel: Placement,
    ) -> Result<Placement, EngineError> {
        let active = VIEWS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(view.label())
            .is_some_and(|state| state.entered);
        let size = if active {
            let window = view.window();
            let scale = window
                .scale_factor()
                .map_err(|e| EngineError::Platform(e.to_string()))?;
            let size = window
                .inner_size()
                .map_err(|e| EngineError::Platform(e.to_string()))?
                .to_logical::<f64>(scale);
            (size.width, size.height)
        } else {
            (panel.bounds.width, panel.bounds.height)
        };
        Ok(VIEWS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(view.label().to_owned())
            .or_default()
            .place(panel, size))
    }

    pub(crate) fn changed(view: &Webview<Wry>, entered: bool) {
        let panel = {
            let mut views = VIEWS.lock().unwrap_or_else(PoisonError::into_inner);
            let state = views.entry(view.label().to_owned()).or_default();
            if state.entered == entered {
                return;
            }
            state.entered = entered;
            state.panel
        };
        if let Some(panel) = panel {
            let view = view.clone();
            // Tauri queues the app window's full-screen change before this callback. Read its resulting
            // client size off the UI thread, then ordinary placement also covers later shell resize reports.
            tauri::async_runtime::spawn(async move {
                if let Err(error) = crate::platform::place(&view, Some(panel)).await {
                    tracing::warn!("browser full-screen placement: {error}");
                }
            });
        }
    }

    pub(crate) fn forget(view: &Webview<Wry>) {
        let active = VIEWS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(view.label())
            .is_some_and(|state| state.entered);
        if active {
            let _ = view.window().set_fullscreen(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_fills_window_through_shell_resizes_and_restores_its_panel() {
        let mut layout = Layout::default();
        let panel = Placement {
            bounds: Rect {
                x: 300.0,
                y: 70.0,
                width: 500.0,
                height: 400.0,
            },
            zoom: 1.25,
        };
        assert_eq!(layout.place(panel, (900.0, 600.0)), panel);
        layout.entered = true;
        let full = layout.place(panel, (1920.0, 1080.0));
        assert_eq!(
            full.bounds,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0
            }
        );
        assert_eq!(full.zoom, panel.zoom);
        assert_eq!(layout.place(panel, (1280.0, 720.0)).bounds.width, 1280.0);
        layout.entered = false;
        assert_eq!(layout.place(layout.panel.unwrap(), (900.0, 600.0)), panel);
    }
}
