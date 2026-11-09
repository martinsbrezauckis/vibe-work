//! Live native input and page evaluation for the Tauri engine. Depends on the mount registry and visibility lock;
//! `PreviewEngine` calls these bounded operations. The OS adapter validates the active window again on the UI thread.

use super::{DRAIN_WAIT, TauriEngine};
use std::sync::{Arc, Mutex, PoisonError};
use vw_preview::{EngineError, MountId, NativeClickGuard, NativeClickPoint};

impl TauriEngine {
    pub(super) fn remember_initializer(&self, spec: &vw_preview::MountSpec) -> String {
        let trusted = spec
            .console_script
            .replace("const TRUSTED = false;", "const TRUSTED = true;");
        self.initializers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(spec.id.label.clone(), trusted.clone());
        trusted
    }
    pub(super) fn isolated_script(&self, id: &MountId, script: &str) -> String {
        let init = self
            .initializers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id.label)
            .cloned()
            .unwrap_or_default();
        format!("{init}\n{script}")
    }
    pub(super) async fn native_click_inner(
        &self,
        id: MountId,
        point: NativeClickPoint,
        guard: NativeClickGuard,
    ) -> Result<(), EngineError> {
        let _visibility = self.visibility.lock().await;
        if !self
            .shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id.label)
            .is_some_and(|shown| shown.visible)
        {
            return Err(EngineError::Platform(
                "Not run: this browser is not in the app's active window; show it there before a native click"
                    .to_owned(),
            ));
        }
        let webview = self.webview(&id)?;
        let known = self
            .persons
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id.label)
            .cloned();
        let person = if let Some(person) = known {
            person
        } else {
            self.watch_person(&webview, &id.label).await?
        };
        crate::native_click::native_click(
            &webview,
            (point.x, point.y),
            (point.viewport_width, point.viewport_height),
            guard,
            person,
        )
        .await
    }

    /// Native frame registries, trusted initialization and pointer watches before the first navigation.
    pub(super) async fn native_watches(
        &self,
        webview: &tauri::Webview<tauri::Wry>,
        label: &str,
    ) -> Result<(), EngineError> {
        crate::agent_frames::install(webview).await?;
        self.install_dialogs(webview, label).await?;
        #[cfg(target_os = "macos")]
        {
            let source = self
                .initializers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(label)
                .cloned()
                .ok_or(EngineError::NotMounted)?;
            crate::mac_agent::install_initializer(webview, source).await?;
        }
        #[cfg(windows)]
        {
            let label = label.to_owned();
            let act_view = webview.clone();
            crate::platform::with_native(
                webview,
                "watching file pickers and full screen",
                std::time::Duration::from_secs(5),
                move |native, reply| {
                    if let Err(error) = crate::frames_win::watch_act(&native, label, act_view) {
                        tracing::warn!("file pickers and full screen are not watched: {error}");
                        // A view without its native refusal watches cannot admit agent input.
                        crate::frames_win::forget(&native);
                    }
                    let _ = reply.send(Ok(()));
                },
            )
            .await?;
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let label = label.to_owned();
            crate::platform::with_native(
                webview,
                "watching file pickers",
                std::time::Duration::from_secs(5),
                move |native, reply| {
                    #[cfg(target_os = "linux")]
                    crate::act_window::install(&native.inner(), label);
                    #[cfg(target_os = "macos")]
                    crate::act_window::install(native.inner() as usize, label);
                    let _ = reply.send(Ok(()));
                },
            )
            .await?;
        }
        let watched = self.watch_person(webview, label).await;
        // Windows: a pointer hook the system refuses must not keep the page from opening. The next checked
        // click tries again and is refused while the watch stays unavailable.
        #[cfg(windows)]
        if let Err(error) = &watched {
            tracing::warn!("the person's pointer is not watched: {error}");
            return Ok(());
        }
        watched.map(drop)
    }

    /// Watches the person's own pointer on this mount's native view from now on (mount, or a first click).
    pub(super) async fn watch_person(
        &self,
        webview: &tauri::Webview<tauri::Wry>,
        label: &str,
    ) -> Result<Arc<vw_preview::PersonPointer>, EngineError> {
        let person = crate::person_watch::install(webview, label).await?;
        self.persons
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(label.to_owned(), person.clone());
        Ok(person)
    }

    /// Native evaluation with a JSON answer (no IPC): the console drain and trusted viewer operations. A page that
    /// navigates away before answering drops the callback: answered as an empty JSON string value.
    pub(super) async fn evaluate_inner(
        &self,
        id: &MountId,
        script: String,
    ) -> Result<String, EngineError> {
        // A cancelled Linux snapshot must finish restoring native zoom before the capture's page cleanup runs.
        #[cfg(target_os = "linux")]
        let _visibility = self.visibility.lock().await;
        let webview = self.webview(id)?;
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let tx = Mutex::new(Some(tx));
        webview
            .eval_with_callback(script, move |json| {
                if let Some(tx) = tx.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    let _ = tx.send(json);
                }
            })
            .map_err(|e| EngineError::Platform(e.to_string()))?;
        match tokio::time::timeout(DRAIN_WAIT, rx).await {
            Ok(Ok(json)) => Ok(json),
            Ok(Err(_)) => Ok("\"\"".to_owned()),
            Err(_) => Err(EngineError::Timeout("evaluating a page script".to_owned())),
        }
    }
}
