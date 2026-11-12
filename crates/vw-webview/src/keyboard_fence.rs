//! Keeps untimestamped native text away from documents born while its command is in flight.
//! The engine installs this before validation and removes it only after the trusted text receipt completes.
use std::time::{Duration, Instant};
use tauri::{Webview, Wry};
use vw_preview::EngineError;

/// Cancellation/error closes the guarded view, so a queued editor command cannot survive cleanup.
pub(crate) struct Fence {
    view: Webview<Wry>,
    source: String,
    token: String,
    armed: bool,
}
impl Drop for Fence {
    fn drop(&mut self) {
        if self.armed {
            // The fence remains installed even if native close fails; never release it on a timeout.
            #[cfg(target_os = "linux")]
            {
                let source = self.source.clone();
                let _ = self.view.with_webview(move |_| {
                    SCRIPTS.with(|scripts| scripts.borrow_mut().remove(&source));
                });
            }
            let _ = crate::agent_frames::forget(&self.view);
            if let Err(error) = self.view.close() {
                tracing::error!("closing interrupted native text: {error}");
            }
        }
    }
}
impl Fence {
    pub(crate) async fn install(view: &Webview<Wry>, token: String) -> Result<Self, EngineError> {
        let fence = Self {
            view: view.clone(),
            source: format!(
                "window.__vwPointGuard.holdKeyboard({});",
                serde_json::json!(token)
            ),
            token,
            armed: true,
        };
        set(&fence.view, fence.source.clone(), true).await?;
        // Keep Drop armed across every await, including cancellation while installing the native script.
        Ok(fence)
    }
    pub(crate) async fn finish(
        mut self,
        page: &str,
        sent: bool,
        person: &vw_preview::PersonInterval,
    ) -> Result<(), EngineError> {
        if sent {
            let source = format!(
                "(()=>{{const r=window[{}];if(r?.token!=={})return false;const a=r.read();return (a.ready===true||a.events?.length===2&&a.events[0].type===\"keydown\"&&a.events[1].type===\"keyup\")&&!a.ambiguous;}})()",
                serde_json::json!(format!("{page}_pointReceipt")),
                serde_json::json!(self.token)
            );
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let answers =
                    crate::agent_frames::evaluate(&self.view, None, source.clone()).await?;
                person
                    .undisturbed()
                    .map_err(|reason| EngineError::Platform(reason.into()))?;
                if answers.iter().any(|a| a.value == "true") {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(EngineError::Platform("Native keyboard input was sent but its document did not confirm completion. Reopen the browser pane and read the result before another action.".into()));
                }
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        }
        set(&self.view, self.source.clone(), false).await?;
        crate::agent_frames::evaluate(
            &self.view,
            None,
            format!(
                "window.__vwPointGuard?.releaseKeyboard({});null",
                serde_json::json!(self.token)
            ),
        )
        .await?;
        self.armed = false;
        Ok(())
    }
}

async fn set(view: &Webview<Wry>, source: String, enabled: bool) -> Result<(), EngineError> {
    crate::platform::with_native(
        view,
        "guarding native text documents",
        Duration::from_secs(5),
        move |native, reply| {
            if reply.is_closed() {
                return;
            }
            #[cfg(target_os = "linux")]
            let result = linux(&native.inner(), &source, enabled);
            #[cfg(target_os = "macos")]
            let result = if enabled {
                vw_platform::webview::agent_world_mac::install_agent_initializer(
                    native.inner(),
                    &source,
                )
            } else {
                vw_platform::webview::agent_world_mac::remove_agent_initializer(
                    native.inner(),
                    &source,
                )
            };
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            let _ = reply.send(result);
            #[cfg(windows)]
            crate::frames_win::keyboard_fence(
                &native,
                enabled.then_some(source),
                Box::new(move |result| {
                    let _ = reply.send(result.map(drop));
                }),
            );
        },
    )
    .await
}

#[cfg(target_os = "linux")]
fn linux(view: &webkit2gtk::WebView, source: &str, enabled: bool) -> Result<(), String> {
    use webkit2gtk::{UserContentManagerExt as _, WebViewExt as _};
    let manager = view
        .user_content_manager()
        .ok_or("native content manager is unavailable")?;
    SCRIPTS.with(|scripts| {
        if enabled {
            let script = webkit2gtk::UserScript::for_world(
                source,
                webkit2gtk::UserContentInjectedFrames::AllFrames,
                webkit2gtk::UserScriptInjectionTime::Start,
                "vibe-work-agent",
                &[],
                &[],
            );
            manager.add_script(&script);
            scripts.borrow_mut().insert(source.to_owned(), script);
        } else if let Some(script) = scripts.borrow_mut().remove(source) {
            manager.remove_script(&script);
        }
    });
    Ok(())
}

#[cfg(target_os = "linux")]
thread_local! { static SCRIPTS: std::cell::RefCell<std::collections::HashMap<String, webkit2gtk::UserScript>> = std::cell::RefCell::default(); }
