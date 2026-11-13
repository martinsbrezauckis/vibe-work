//! Scoped `WebView2` mouse variants using its own protocol channel after native HWND/focus checks.
//! Called by the checked frame input adapter; runs the steps of `windows_sequence_plan` in order. Every admitted
//! step rechecks the deadline, permission and foreground target; every press is released.
use super::windows_sequence_plan::{self as plan, Step};
use super::{ClickPermission, ClickReply};
use crate::webview::devtools_win::DevTools;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;

struct Sequence {
    controller: ICoreWebView2Controller,
    channel: DevTools,
    point: (f64, f64),
    css: (f64, f64),
    options: plan::Options,
    steps: Vec<Step>,
    at: Cell<usize>,
    sent: Arc<AtomicBool>,
    allowed: ClickPermission,
    deadline: std::time::Instant,
    /// Native event creation time, preserved when the browser's protocol queue is busy.
    timestamp: f64,
    reply: RefCell<Option<ClickReply>>,
}

/// Delivers a bounded sequence through this controller's browser process only.
/// Tuple arguments are normalized point, CSS point and DOM button/count/modifier bits.
pub fn click(
    controller: &ICoreWebView2Controller,
    points: ((f64, f64), (f64, f64)),
    options: plan::Options,
    sent: Arc<AtomicBool>,
    allowed: ClickPermission,
    deadline: std::time::Instant,
    reply: ClickReply,
) {
    if !plan::admissible(options, points.1) {
        let _ = reply.send(Err("invalid native click options".into()));
        return;
    }
    let channel = match DevTools::new(controller) {
        Ok(channel) => channel,
        Err(error) => {
            let _ = reply.send(Err(error));
            return;
        }
    };
    let sequence = Rc::new(Sequence {
        controller: controller.clone(),
        channel,
        point: points.0,
        css: points.1,
        options,
        steps: plan::steps(options),
        at: Cell::new(0),
        sent,
        allowed,
        deadline,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64(),
        reply: RefCell::new(Some(reply)),
    });
    sequence.next();
}

impl Sequence {
    fn finish(&self, result: Result<(), String>) {
        if let Some(reply) = self.reply.borrow_mut().take() {
            let _ = reply.send(result);
        }
    }
    /// The live checks before a step that starts input; `false` after it answered (or the caller went away).
    fn admit(&self) -> bool {
        if self
            .reply
            .borrow()
            .as_ref()
            .is_none_or(ClickReply::is_closed)
        {
            return false;
        }
        if std::time::Instant::now() >= self.deadline {
            self.finish(Err(
                "the page moved: native input preparation took too long; read the page again"
                    .into(),
            ));
            return false;
        }
        if self.allowed.as_ref().is_some_and(|allowed| !allowed()) {
            self.finish(Err("Browser permission changed before native input.".into()));
            return false;
        }
        if let Err(error) = super::windows_click::target(&self.controller, self.point) {
            self.finish(Err(error));
            return false;
        }
        true
    }
    fn next(self: Rc<Self>) {
        let Some(&step) = self.steps.get(self.at.get()) else {
            self.finish(Ok(()));
            return;
        };
        if step.admitted() && !self.admit() {
            return;
        }
        if step.sends() {
            self.sent.store(true, Ordering::Release);
        }
        // After a slow press callback a release goes outside the viewport (see `plan::event`).
        let late = std::time::Instant::now() >= self.deadline;
        let event = plan::event(step, self.css, self.options, self.timestamp, late).to_string();
        let next = self.clone();
        self.channel.call(
            "",
            "Input.dispatchMouseEvent",
            &event,
            Box::new(move |result| {
                if let Err(error) = result {
                    next.finish(Err(match step {
                        Step::Approach => {
                            format!("Native pointer move failed before input: {error}")
                        }
                        Step::Release(_) => {
                            format!("Native mouse release failed after input was sent: {error}")
                        }
                        _ => format!("Native mouse press failed: {error}"),
                    }));
                    return;
                }
                if step.sends() {
                    next.sent.store(true, Ordering::Release);
                }
                // A press is always followed by its release, also when Stop or a permission change arrived
                // after the press: only admitted steps recheck.
                next.at.set(next.at.get() + 1);
                next.next();
            }),
        );
    }
}
