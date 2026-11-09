//! Scoped `WebView2` mouse variants using its own protocol channel after native HWND/focus checks.
//! Called by the checked frame input adapter; every press rechecks permission and cancellation, every press is released.
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
    options: (u8, u8, u8),
    count: Cell<u8>,
    sent: Arc<AtomicBool>,
    allowed: ClickPermission,
    reply: RefCell<Option<ClickReply>>,
}

/// Delivers a bounded sequence through this controller's browser process only.
/// Tuple arguments are normalized point, CSS point and DOM button/count/modifier bits.
pub fn click(
    controller: &ICoreWebView2Controller,
    points: ((f64, f64), (f64, f64)),
    options: (u8, u8, u8),
    sent: Arc<AtomicBool>,
    allowed: ClickPermission,
    reply: ClickReply,
) {
    if (options != (0, 0, 0) && (options.0 > 2 || !(1..=3).contains(&options.1) || options.2 > 15))
        || ![points.1.0, points.1.1]
            .into_iter()
            .all(|n| n.is_finite() && n >= 0.0)
    {
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
        count: Cell::new(1),
        sent,
        allowed,
        reply: RefCell::new(Some(reply)),
    });
    sequence.press();
}

impl Sequence {
    fn finish(&self, result: Result<(), String>) {
        if let Some(reply) = self.reply.borrow_mut().take() {
            let _ = reply.send(result);
        }
    }
    fn event(&self, kind: &str) -> String {
        if self.options.1 == 0 {
            return serde_json::json!({"type":"mouseMoved","x":self.css.0,"y":self.css.1,"button":"none","buttons":0}).to_string();
        }
        let button = match self.options.0 {
            1 => "middle",
            2 => "right",
            _ => "left",
        };
        serde_json::json!({"type":kind,"x":self.css.0,"y":self.css.1,"button":button,"clickCount":self.count.get(),"modifiers":self.options.2}).to_string()
    }
    fn press(self: Rc<Self>) {
        if self
            .reply
            .borrow()
            .as_ref()
            .is_none_or(ClickReply::is_closed)
        {
            return;
        }
        if self.allowed.as_ref().is_some_and(|allowed| !allowed()) {
            self.finish(Err("Browser permission changed before native input.".into()));
            return;
        }
        if let Err(error) = super::windows_click::target(&self.controller, self.point) {
            self.finish(Err(error));
            return;
        }
        let next = self.clone();
        self.sent.store(true, Ordering::Release);
        self.channel.call(
            "",
            "Input.dispatchMouseEvent",
            &self.event("mousePressed"),
            Box::new(move |result| {
                if let Err(error) = result {
                    next.finish(Err(format!("Native mouse press failed: {error}")));
                    return;
                }
                next.sent.store(true, Ordering::Release);
                if next.options.1 == 0 {
                    next.finish(Ok(()));
                    return;
                }
                // Release even when Stop or a permission change arrived after the press.
                next.release();
            }),
        );
    }
    fn release(self: Rc<Self>) {
        let next = self.clone();
        self.channel.call(
            "",
            "Input.dispatchMouseEvent",
            &self.event("mouseReleased"),
            Box::new(move |result| {
                if let Err(error) = result {
                    next.finish(Err(format!(
                        "Native mouse release failed after input was sent: {error}"
                    )));
                    return;
                }
                if next.count.get() >= next.options.1 {
                    next.finish(Ok(()));
                } else {
                    next.count.set(next.count.get() + 1);
                    next.press();
                }
            }),
        );
    }
}
