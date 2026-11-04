//! Windows frame evaluation: every frame's private script world through the `WebView2` `DevTools` protocol
//! (vw-preview `CdpFrames`), with a host-owned registry generation, so frame reads, final click validation and
//! the event receipt run where page scripts cannot reach. A runtime that refuses the protocol fails closed for reads and input.
//!
//! Depends on: vw-platform `devtools_win`, vw-preview `CdpFrames`. Called by `agent_frames`
//! and `frame_click` on the webview's UI thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::Ordering;

use tauri::webview::PlatformWebview;
use vw_platform::webview::devtools_win;
use vw_preview::{CDP_EVENTS, CdpDone, CdpFrames, CdpTransport, FramesDone};

struct Channel {
    tools: devtools_win::DevTools,
    controller: vw_platform::webview::devtools_win::Controller,
}

impl CdpTransport for Channel {
    fn close(&self) -> Result<(), String> {
        devtools_win::close(&self.controller)
    }
    fn call(&self, session: &str, method: &str, params: serde_json::Value, done: CdpDone) {
        self.tools.call(
            session,
            method,
            &params.to_string(),
            Box::new(move |answer| {
                done(
                    answer.and_then(|json| serde_json::from_str(&json).map_err(|e| e.to_string())),
                );
            }),
        );
    }
}

thread_local! {
    /// Isolated-world registries by webview; a webview missing here cannot be read.
    static VIEWS: RefCell<HashMap<usize, CdpFrames>> = RefCell::new(HashMap::new());
    static COSMETICS: RefCell<HashMap<usize, vw_preview::shields::cosmetic::Cosmetics>> = RefCell::new(HashMap::new());
}

fn registry(key: usize) -> Option<CdpFrames> {
    VIEWS.with(|views| views.borrow().get(&key).cloned())
}

/// Installs the frame registry before the first admitted navigation. Completion means it is in place.
pub(crate) fn install(native: &PlatformWebview, done: Box<dyn FnOnce(Result<(), String>)>) {
    let controller = native.controller();
    let key = devtools_win::view_key(&controller);
    let isolated = devtools_win::DevTools::new(&controller).and_then(|tools| {
        tools.listen(
            &CDP_EVENTS,
            Box::new(move |session, method, params| {
                let params = serde_json::from_str(params).unwrap_or_default();
                if let Some(frames) = registry(key) {
                    frames.on_event(session, method, &params);
                }
                let cosmetics = COSMETICS.with(|views| views.borrow().get(&key).cloned());
                if let Some(cosmetics) = cosmetics {
                    cosmetics.on_event(session, method, &params);
                }
            }),
        )?;
        let channel = Rc::new(Channel {
            tools,
            controller: controller.clone(),
        });
        COSMETICS.with(|views| {
            views.borrow_mut().insert(
                key,
                vw_preview::shields::cosmetic::Cosmetics::new(channel.clone()),
            )
        });
        Ok(CdpFrames::new(channel))
    });
    let refuse = move |reason: String, done: Box<dyn FnOnce(Result<(), String>)>| {
        tracing::warn!("isolated browser frames unavailable: {reason}");
        VIEWS.with(|views| views.borrow_mut().remove(&key));
        COSMETICS.with(|views| views.borrow_mut().remove(&key));
        done(Err(format!(
            "isolated browser worlds are unavailable: {reason}"
        )));
    };
    match isolated {
        Ok(frames) => {
            VIEWS.with(|views| views.borrow_mut().insert(key, frames.clone()));
            frames.install(Box::new(move |result| match result {
                Ok(()) => done(Ok(())),
                Err(reason) => refuse(reason, done),
            }));
        }
        Err(reason) => refuse(reason, done),
    }
}

/// Evaluates in all frames (`None`) or one frame id, in the agent's private world.
pub(crate) fn evaluate(
    native: &PlatformWebview,
    frame: Option<&str>,
    script: &str,
    done: FramesDone,
) {
    let controller = native.controller();
    if let Some(frames) = registry(devtools_win::view_key(&controller)) {
        frames.evaluate(frame, script, done);
        return;
    }
    done(Err("isolated browser worlds are unavailable".into()));
}

/// [`evaluate`] for an agent act's own input: a user gesture in the private world.
pub(crate) fn evaluate_input(
    native: &PlatformWebview,
    frame: Option<&str>,
    script: &str,
    done: FramesDone,
) {
    let controller = native.controller();
    if let Some(frames) = registry(devtools_win::view_key(&controller)) {
        frames.evaluate_input(frame, script, done);
        return;
    }
    evaluate(native, frame, script, done);
}

/// Starts an agent act's window on the view (every input: gesture evaluation or native click): the file picker
/// interception, so a picker the page opens never shows (the engine reports it, `watch_act`), and focus emulation,
/// so the page counts as focused for the act's click (a Copy button's clipboard write works, as for a person's
/// click) while the person's keyboard focus stays where it is.
pub(crate) fn start_act(
    native: &PlatformWebview,
    generation: u64,
    done: Box<dyn FnOnce(Result<(), String>)>,
) {
    if let Some(frames) = registry(devtools_win::view_key(&native.controller())) {
        frames.start_act(generation, done);
    } else {
        done(Err("Not run: browser input restrictions are unavailable. Reopen the browser view before trying again.".into()));
    }
}

/// Ends an agent act only after every disable or session teardown completes.
pub(crate) fn end_act(
    native: &PlatformWebview,
    generation: u64,
    done: Box<dyn FnOnce(Result<(), String>)>,
) {
    if let Some(frames) = registry(devtools_win::view_key(&native.controller())) {
        frames.end_act(generation, done);
    } else {
        done(Ok(()));
    }
}

/// A missing completion cannot keep the session's emulation alive beyond its window.
pub(crate) fn abort_act(native: &PlatformWebview, generation: u64) {
    if let Some(frames) = registry(devtools_win::view_key(&native.controller())) {
        frames.abort_act(
            generation,
            Box::new(|result| {
                if let Err(error) = result {
                    tracing::error!("browser control teardown: {error}");
                }
            }),
        );
    }
}

/// Rechecks admission immediately before a checked native click is posted.
pub(crate) fn act_ready(native: &PlatformWebview) -> Result<(), String> {
    registry(devtools_win::view_key(&native.controller()))
        .ok_or_else(|| {
            "Not run: browser input restrictions are unavailable. Reopen the browser view."
                .to_owned()
        })?
        .act_ready()
}

/// Reports every picker the engine intercepted (only during agent acts) as refused, and leaves full screen at once
/// when a page entered it during an agent act (`WebView2` cannot refuse it before it starts).
pub(crate) fn watch_act(
    native: &PlatformWebview,
    label: String,
    webview: tauri::Webview<tauri::Wry>,
) -> Result<(), String> {
    let controller = native.controller();
    let key = devtools_win::view_key(&controller);
    let pickers = label.clone();
    devtools_win::DevTools::new(&controller)?.listen(
        &["Page.fileChooserOpened"],
        Box::new(move |_, _, _| crate::act_window::picker_refused(&pickers)),
    )?;
    let exiting = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    vw_platform::webview::full_screen_win::on_change(
        &controller,
        Box::new(move |entered| {
            let refused = entered && crate::act_window::full_screen_entered(&label);
            if !refused {
                crate::full_screen::changed(&webview, entered);
            }
            if refused && registry(key).is_some() {
                if exiting.swap(true, Ordering::Relaxed) {
                    return;
                }
                let (webview, exiting) = (webview.clone(), exiting.clone());
                tauri::async_runtime::spawn(async move {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    loop {
                        let remaining =
                            deadline.saturating_duration_since(std::time::Instant::now());
                        if remaining.is_zero() {
                            break;
                        }
                        let result = crate::platform::with_native(
                            &webview,
                            "leaving browser full screen",
                            remaining,
                            move |native, reply| {
                                let controller = native.controller();
                                let Some(frames) = registry(devtools_win::view_key(&controller))
                                else {
                                    let _ = reply.send(Ok(false));
                                    return;
                                };
                                frames.exit_full_screen(
                                    Rc::new(move || {
                                        vw_platform::webview::full_screen_win::contains(&controller)
                                    }),
                                    Box::new(move |result| {
                                        let _ = reply.send(result);
                                    }),
                                );
                            },
                        )
                        .await;
                        match result {
                            Ok(false) => break,
                            Ok(true) | Err(vw_preview::EngineError::Timeout(_)) => {}
                            Err(error) => {
                                tracing::warn!("browser full-screen exit: {error}");
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    exiting.store(false, Ordering::Relaxed);
                });
            }
        }),
    )
}

/// Drops the webview's registries when it closes.
pub(crate) fn forget(native: &PlatformWebview) {
    let controller = native.controller();
    VIEWS.with(|views| {
        views
            .borrow_mut()
            .remove(&devtools_win::view_key(&controller))
    });
    COSMETICS.with(|views| {
        views
            .borrow_mut()
            .remove(&devtools_win::view_key(&controller))
    });
}

/// Updates the native inspector sheets in every known frame; no page-world fallback.
pub(crate) fn update_cosmetics(
    native: &PlatformWebview,
    matcher: std::sync::Arc<vw_preview::shields::matcher::Matcher>,
    done: Box<dyn FnOnce(Result<(), String>)>,
) {
    let key = devtools_win::view_key(&native.controller());
    let cosmetics = COSMETICS.with(|views| views.borrow().get(&key).cloned());
    match cosmetics {
        Some(cosmetics) => cosmetics.update(matcher, done),
        None if matcher.is_empty() => done(Ok(())),
        None => done(Err(
            "Browser ad hiding requires the native frame channel".into()
        )),
    }
}
