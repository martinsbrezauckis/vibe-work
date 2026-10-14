//! The app window for the Debug API: `/windows`, `/screenshot` and `/window` on Tauri. Only wiring:
//! the captures are vw-webview's (`window_shot`), the routes vw-debug-api's (`window_routes`).
//!
//! Depends on: tauri, vw-webview (`window_shot`), vw-debug-api (`AppWindow`). Called by: `startup.rs` (passed to
//! the host, which hands it to the Debug API).
//! Invariants:
//! - Only the window labelled `main` and this process's own dialogs are touched; the window is looked up for
//!   every request (the host starts before the window exists).
//! - Size requests are clamped to the configured minimum by the OS; the answer reports the size it got.
//!   Debug test builds may opt into a 390 px minimum with `VW_E2E_NARROW_WINDOW=1` for responsive checks.
//! - Test builds only (feature `e2e-script`): `script` evaluates the Debug API's wrapped script in the
//!   main webview (Tauri `eval_with_callback`) and waits at most 30 s for the engine's answer; `e2e_windows` lists
//!   the webview windows' labels.

use tauri::{AppHandle, LogicalSize, Manager, Size};
use vw_debug_api::{AppWindow, BoxFuture, WindowRequest, WindowState, WindowView};

/// The main window's label.
const MAIN: &str = "main";

/// The [`AppWindow`] of the running app.
pub struct TauriWindow {
    app: AppHandle,
}

impl TauriWindow {
    /// For `app`'s main window.
    #[must_use]
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn window(&self) -> Result<tauri::Window, String> {
        self.app
            .get_window(MAIN)
            .ok_or_else(|| "the main window is not open".to_owned())
    }

    fn min_size(&self) -> (f64, f64) {
        let (width, height) = self
            .app
            .config()
            .app
            .windows
            .iter()
            .find(|w| w.label == MAIN)
            .map_or((0.0, 0.0), |w| {
                (w.min_width.unwrap_or(0.0), w.min_height.unwrap_or(0.0))
            });
        #[cfg(all(debug_assertions, feature = "e2e-script"))]
        if std::env::var("VW_E2E_NARROW_WINDOW").as_deref() == Ok("1") {
            return (width.min(390.0), height);
        }
        (width, height)
    }

    fn state(&self, window: &tauri::Window) -> Result<WindowState, String> {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let size = window
            .inner_size()
            .map_err(|e| e.to_string())?
            .to_logical::<f64>(scale);
        let (min_width, min_height) = self.min_size();
        Ok(WindowState {
            width: size.width,
            height: size.height,
            scale,
            maximized: window.is_maximized().unwrap_or(false),
            minimized: window.is_minimized().unwrap_or(false),
            focused: window.is_focused().unwrap_or(false),
            min_width,
            min_height,
        })
    }
}

impl AppWindow for TauriWindow {
    fn windows(&self) -> BoxFuture<'_, Result<Vec<WindowView>, String>> {
        Box::pin(async move {
            let window = self.window()?;
            Ok(vw_webview::window_shot::windows(&window)
                .await?
                .into_iter()
                .map(|w| WindowView {
                    id: w.id,
                    kind: w.kind.to_owned(),
                    title: w.title,
                    width: w.width,
                    height: w.height,
                })
                .collect())
        })
    }

    fn capture(&self, id: String) -> BoxFuture<'_, Result<Vec<u8>, String>> {
        Box::pin(async move {
            let window = self.window()?;
            if window.is_minimized().map_err(|e| e.to_string())?
                || !window.is_visible().map_err(|e| e.to_string())?
            {
                return Err("Not run: the Vibe Work window is minimized or hidden. Show the window, then retry the screenshot.".to_owned());
            }
            vw_webview::window_shot::capture(&window, &id).await
        })
    }

    fn control(&self, request: WindowRequest) -> BoxFuture<'_, Result<WindowState, String>> {
        Box::pin(async move {
            let window = self.window()?;
            let done = |r: tauri::Result<()>| r.map_err(|e| e.to_string());
            match request {
                WindowRequest::State {} => {}
                WindowRequest::Size { width, height } => {
                    #[cfg(all(debug_assertions, feature = "e2e-script"))]
                    if std::env::var("VW_E2E_NARROW_WINDOW").as_deref() == Ok("1") {
                        let (min_width, min_height) = self.min_size();
                        done(window.set_min_size(Some(Size::Logical(LogicalSize::new(
                            min_width, min_height,
                        )))))?;
                    }
                    if window.is_maximized().unwrap_or(false) {
                        done(window.unmaximize())?;
                    }
                    done(window.set_size(Size::Logical(LogicalSize::new(width, height))))?;
                }
                WindowRequest::Maximize {} => done(window.maximize())?,
                WindowRequest::Restore {} => {
                    done(window.unminimize())?;
                    done(window.unmaximize())?;
                }
                WindowRequest::Minimize {} => done(window.minimize())?,
                WindowRequest::Focus {} => {
                    done(window.unminimize())?;
                    done(window.set_focus())?;
                }
            }
            // The OS applies a size or state change asynchronously: give it a moment before reporting.
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            self.state(&window)
        })
    }

    #[cfg(feature = "e2e-script")]
    fn script(&self, js: String) -> BoxFuture<'_, Result<String, String>> {
        Box::pin(async move {
            let webview = self
                .app
                .get_webview(MAIN)
                .ok_or_else(|| "the main webview is not open".to_owned())?;
            let (sender, answer) = tokio::sync::oneshot::channel::<String>();
            // The engine calls back once (WebKitGTK `FnOnce`); the API takes an `Fn`, so the sender sits in a slot.
            let slot = std::sync::Mutex::new(Some(sender));
            webview
                .eval_with_callback(js, move |result| {
                    if let Some(sender) = slot.lock().ok().and_then(|mut s| s.take()) {
                        let _ = sender.send(result);
                    }
                })
                .map_err(|e| e.to_string())?;
            match tokio::time::timeout(std::time::Duration::from_secs(30), answer).await {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(_)) => Err("the engine dropped the script's answer".to_owned()),
                Err(_) => Err(vw_debug_api::e2e_script::TIMEOUT.to_owned()),
            }
        })
    }

    #[cfg(feature = "e2e-script")]
    fn e2e_windows(&self) -> BoxFuture<'_, Result<Vec<String>, String>> {
        Box::pin(async move { Ok(self.app.webview_windows().into_keys().collect()) })
    }
}
