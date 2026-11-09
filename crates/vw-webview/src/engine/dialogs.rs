//! Connects the shared native dialog hub to preview mounts and their live ownership.
//! All native continuations run on the UI thread; tool reads only copy bounded metadata.
use super::TauriEngine;
use std::{sync::Arc, time::Duration};
use vw_preview::{DialogAnswer, EngineError, MountId};

impl TauriEngine {
    pub(super) fn update_dialog_ownership(
        &self,
        id: &MountId,
        owned: bool,
    ) -> Result<(), EngineError> {
        let view = match self.webview(id) {
            Ok(view) => view,
            Err(EngineError::NotMounted) => return Ok(()),
            Err(error) => return Err(error),
        };
        let hub = self.dialogs.clone();
        let label = id.label.clone();
        view.with_webview(move |native| {
            if !owned {
                vw_platform::webview::dialogs::release(&hub, &label);
            }
            #[cfg(windows)]
            if let Err(error) =
                vw_platform::webview::dialogs::ownership(&native.controller(), owned)
            {
                tracing::warn!("{error}");
            }
            #[cfg(not(windows))]
            let _ = native;
        })
        .map_err(|e| EngineError::Platform(e.to_string()))
    }

    pub(super) async fn install_dialogs(
        &self,
        webview: &tauri::Webview<tauri::Wry>,
        label: &str,
    ) -> Result<(), EngineError> {
        let hooks = self.hooks()?;
        let label = label.to_owned();
        let owned = label.clone();
        let admit: vw_platform::webview::dialogs::Admit =
            Arc::new(move || hooks.handles_dialog(&owned));
        let hub = self.dialogs.clone();
        crate::platform::with_native(
            webview,
            "watching page dialogs",
            Duration::from_secs(5),
            move |native, reply| {
                #[cfg(target_os = "linux")]
                let result =
                    vw_platform::webview::dialogs::install(&native.inner(), label, hub, admit);
                #[cfg(windows)]
                let result =
                    vw_platform::webview::dialogs::install(&native.controller(), label, hub, admit);
                #[cfg(target_os = "macos")]
                let result = vw_platform::webview::dialogs::install(
                    native.inner() as usize,
                    label,
                    hub,
                    admit,
                );
                let _ = reply.send(result);
            },
        )
        .await
    }

    pub(super) async fn answer_dialog_inner(
        &self,
        id: &MountId,
        dialog: String,
        answer: DialogAnswer,
    ) -> Result<(), EngineError> {
        let view = self.webview(id)?;
        let label = id.label.clone();
        let hub = self.dialogs.clone();
        crate::platform::with_native(
            &view,
            "answering page dialog",
            Duration::from_secs(5),
            move |_native, reply| {
                let answer = vw_platform::webview::dialogs::Answer {
                    accept: answer.accept,
                    text: answer.text,
                    credentials: answer.credentials,
                };
                let result = vw_platform::webview::dialogs::answer(&hub, &label, &dialog, answer);
                let _ = reply.send(result);
            },
        )
        .await
    }
}
