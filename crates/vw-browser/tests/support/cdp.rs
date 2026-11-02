//! The fixture's `DevTools` route (`VW_TEST_CDP_FRAMES=1`): the production isolated-world frame registry over the
//! page target's raw protocol channel, as the `WebView2` adapter runs it, with the act window's file picker
//! interception. Depends on the fixture pipe (`super`); used by the page engine in `mod.rs`.
#![allow(dead_code)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use serde_json::{Value, json};

use super::{Pipe, rpc_blocking};
/// One frame evaluation for the isolated-world registry thread.
pub(super) enum CdpRequest {
    /// One evaluation; `input`: an agent act's own input (a user gesture, as the `WebView2` adapter).
    Evaluate(
        Option<String>,
        String,
        tokio::sync::oneshot::Sender<Result<Vec<vw_preview::FrameAnswer>, String>>,
        bool,
    ),
    /// The act window's file picker interception on or off (the `WebView2` adapter does this for every act: a
    /// gesture evaluation or a native click). The answer comes once the browser confirmed it.
    Intercept(bool, tokio::sync::oneshot::Sender<()>),
}

/// File pickers Chromium reported while interception was on (`Page.fileChooserOpened`).
pub(super) static PICKERS: AtomicU64 = AtomicU64::new(0);
/// Test switch: a native click without the act window's interception (the route before every act had one).
pub static NATIVE_CLICK_WITHOUT_WINDOW: AtomicBool = AtomicBool::new(false);

/// The page target's raw `DevTools` channel, as the `WebView2` adapter's: protocol events reach the registry in
/// the order they were sent, before the answer that followed them.
struct PipeCdp {
    pipe: Arc<Mutex<Pipe>>,
    frames: std::cell::RefCell<Option<vw_preview::CdpFrames>>,
}

impl vw_preview::CdpTransport for PipeCdp {
    fn call(&self, session: &str, method: &str, params: Value, done: vw_preview::CdpDone) {
        let reply = rpc_blocking(
            &self.pipe,
            &json!({"op":"cdp", "session":session, "method":method, "params":params}),
        );
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => return done(Err(error.to_string())),
        };
        for event in reply["events"].as_array().into_iter().flatten() {
            if event["method"] == "Page.fileChooserOpened" {
                PICKERS.fetch_add(1, Ordering::Relaxed);
            }
        }
        let frames = self.frames.borrow().clone();
        if let Some(frames) = frames {
            for event in reply["events"].as_array().into_iter().flatten() {
                frames.on_event(
                    event["session"].as_str().unwrap_or_default(),
                    event["method"].as_str().unwrap_or_default(),
                    &event["params"],
                );
            }
        }
        match reply["error"].as_str() {
            Some(error) => done(Err(error.to_owned())),
            None => done(Ok(reply["result"].clone())),
        }
    }
}

/// The production isolated-world frame registry on a thread of its own (the engine's UI thread on Windows).
pub(super) fn cdp_worker(pipe: Arc<Mutex<Pipe>>) -> std::sync::mpsc::Sender<CdpRequest> {
    let (sender, requests) = std::sync::mpsc::channel::<CdpRequest>();
    std::thread::spawn(move || {
        let transport = std::rc::Rc::new(PipeCdp {
            pipe,
            frames: std::cell::RefCell::new(None),
        });
        let frames = vw_preview::CdpFrames::new(transport.clone());
        *transport.frames.borrow_mut() = Some(frames.clone());
        let (sent, installed) = std::sync::mpsc::channel();
        frames.install(Box::new(move |result| {
            let _ = sent.send(result);
        }));
        let installed = installed.recv().unwrap();
        for request in requests {
            let (target, script, reply, input) = match request {
                CdpRequest::Evaluate(target, script, reply, input) => {
                    (target, script, reply, input)
                }
                CdpRequest::Intercept(on, done) => {
                    frames.intercept_file_choosers(on, Box::new(|r| r.unwrap()));
                    let _ = done.send(());
                    continue;
                }
            };
            if let Err(error) = &installed {
                let _ = reply.send(Err(error.clone()));
                continue;
            }
            if !input {
                frames.evaluate(
                    target.as_deref(),
                    &script,
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
                continue;
            }
            // The transport answers synchronously: the act runs, then interception ends (that call's answer
            // brings the picker event), then the act's answer goes back.
            frames.intercept_file_choosers(true, Box::new(|r| r.unwrap()));
            let (sent, answer) = std::sync::mpsc::channel();
            frames.evaluate_input(
                target.as_deref(),
                &script,
                Box::new(move |result| {
                    let _ = sent.send(result);
                }),
            );
            frames.intercept_file_choosers(false, Box::new(|r| r.unwrap()));
            let _ = reply.send(answer.recv().unwrap_or_else(|e| Err(e.to_string())));
        }
    });
    sender
}

/// `VW_TEST_CDP_FRAMES=1`: frame evaluation (except page-world fixtures) runs through the `DevTools` isolated-world registry the
/// Windows engine uses, instead of the fixture's own helper.
pub(super) fn cdp_frames() -> bool {
    std::env::var_os("VW_TEST_CDP_FRAMES").is_some_and(|v| v == "1")
}
