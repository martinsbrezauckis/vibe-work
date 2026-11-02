//! Isolated all-frame validation, receipt arming and dispatch marking before native input.
//! Called by `native_click`; Stop, permission and the person's pointer are rechecked after each asynchronous
//! engine callback.
use std::sync::{Arc, atomic::AtomicBool};
use tauri::webview::PlatformWebview;
#[cfg(not(windows))]
use vw_preview::FrameAnswer;
use vw_preview::{PersonInterval, PersonPointer};

/// Host-only state of one checked click, from the preview service's guard and the view's pointer watch.
pub(crate) struct Input {
    pub sent: Arc<AtomicBool>,
    pub allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    pub person: Arc<PersonPointer>,
    pub interval: Arc<PersonInterval>,
}

#[cfg(not(windows))]
pub(crate) fn click(
    native: PlatformWebview,
    point: (f64, f64),
    request: serde_json::Value,
    input: Input,
    reply: crate::platform::Reply<()>,
) {
    let Input {
        sent,
        allowed,
        person,
        interval,
    } = input;
    let (Some(script), Some(dispatch)) = (request["script"].as_str(), request["dispatch"].as_str())
    else {
        let _ = reply.send(Err("invalid point validation or dispatch mark".into()));
        return;
    };
    // Before the validation script arms the receipt: from here to its final collection, any press or release
    // of the person's in this view means the click is not confirmed.
    person.arm(&interval);
    let (script, dispatch) = (script.to_owned(), dispatch.to_owned());
    let view = native.inner();
    #[cfg(target_os = "linux")]
    let view = &view;
    evaluate(
        view,
        script,
        Box::new(move |answers| {
            if reply.is_closed() {
                return;
            }
            if let Err(reason) =
                answers.and_then(|a| vw_preview::validate_frame_point(&request, &a))
            {
                let _ = reply.send(Err(reason));
                return;
            }
            if allowed.as_ref().is_some_and(|check| !check()) {
                let _ = reply.send(Err(
                    "Not run: browser permission changed before native input".into(),
                ));
                return;
            }
            // The mark is in the receipt's isolated world and clock domain, after validation and before submit.
            // Earlier or concurrent user input remains in that receipt until collection.
            let view = native.inner();
            #[cfg(target_os = "linux")]
            let view = &view;
            evaluate(
                view,
                dispatch,
                Box::new(move |answers| {
                    if reply.is_closed() {
                        return;
                    }
                    if let Err(reason) =
                        answers.and_then(|a| vw_preview::validate_frame_point(&request, &a))
                    {
                        let _ = reply.send(Err(reason));
                        return;
                    }
                    let submit = move || {
                        if reply.is_closed() {
                            return;
                        }
                        if allowed.as_ref().is_some_and(|check| !check()) {
                            let _ = reply.send(Err(
                                "Not run: browser permission changed before native input".into(),
                            ));
                            return;
                        }
                        // Nothing is sent while the person is pressing, has just pressed or released, or
                        // pressed since arming.
                        #[cfg(target_os = "linux")]
                        let admitted = person.admit(
                            &interval,
                            vw_platform::native_click::person::holding(&native.inner()),
                            vw_platform::native_click::person::recent(),
                        );
                        #[cfg(target_os = "macos")]
                        let admitted = person.admit(
                            &interval,
                            vw_platform::native_click::person::holding(native.inner()),
                            vw_platform::native_click::person::recent(),
                        );
                        if let Err(reason) = admitted {
                            let _ = reply.send(Err(reason));
                            return;
                        }
                        #[cfg(target_os = "linux")]
                        let result =
                            vw_platform::native_click::linux_click(&native.inner(), point, &sent);
                        #[cfg(target_os = "macos")]
                        let result =
                            vw_platform::native_click::macos_click(native.inner(), point, &sent);
                        let _ = reply.send(result);
                    };
                    // WebKit (GTK and macOS) floors DOM event times and the mark's clock to whole
                    // milliseconds. Submit at least 2 ms after the mark so our own input is strictly later;
                    // equal timestamps still fail closed as ambiguous. The receipt stays armed and
                    // Stop/permission/the person's pointer are checked inside the delayed callback. A frame
                    // created or changed meanwhile invalidates the receipt (native frame generations).
                    let delay = std::time::Duration::from_millis(2);
                    #[cfg(target_os = "linux")]
                    glib::timeout_add_local_once(delay, submit);
                    #[cfg(target_os = "macos")]
                    vw_platform::native_click::macos_after(delay, submit);
                }),
            );
        }),
    );
}

#[cfg(not(windows))]
// Linux hands the script to WebKitGTK; macOS only borrows it.
#[cfg_attr(target_os = "macos", allow(clippy::needless_pass_by_value))]
fn evaluate(
    #[cfg(target_os = "linux")] view: &webkit2gtk::WebView,
    #[cfg(target_os = "macos")] view: *mut std::ffi::c_void,
    script: String,
    done: Box<dyn FnOnce(Result<Vec<FrameAnswer>, String>)>,
) {
    #[cfg(target_os = "linux")]
    crate::linux_agent::evaluate_native(view, None, script, done);
    #[cfg(target_os = "macos")]
    vw_platform::webview::agent_world_mac::evaluate_in_frames(
        view,
        &script,
        Box::new(move |result| {
            done(result.map(|answers| {
                answers
                    .into_iter()
                    .filter_map(|a| {
                        a.value.ok().map(|value| FrameAnswer {
                            id: a.id,
                            generation: a.generation,
                            main: a.main,
                            url: a.url,
                            origin: None,
                            value,
                        })
                    })
                    .collect()
            }));
        }),
    );
}

/// Every answer comes from the private world registry (it carries the host's generation); the older
/// page-world reads never authorize native input.
#[cfg(windows)]
fn isolated(answers: Vec<vw_preview::FrameAnswer>) -> Result<Vec<vw_preview::FrameAnswer>, String> {
    if answers.is_empty() || answers.iter().any(|a| a.generation.is_none()) {
        return Err(
            "Not run: checked native clicks require isolated frame validation on this platform"
                .into(),
        );
    }
    Ok(answers)
}

/// Windows: the same steps in each frame's private world through the `DevTools` protocol. The click is posted
/// to the browser window right after the mark, on the UI thread, once Stop, permission and the person's pointer
/// allow it.
#[cfg(windows)]
pub(crate) fn click(
    native: PlatformWebview,
    point: (f64, f64),
    request: serde_json::Value,
    input: Input,
    reply: crate::platform::Reply<()>,
) {
    let Input {
        sent,
        allowed,
        person,
        interval,
    } = input;
    let (Some(script), Some(dispatch)) = (request["script"].as_str(), request["dispatch"].as_str())
    else {
        let _ = reply.send(Err("invalid point validation or dispatch mark".into()));
        return;
    };
    let (script, dispatch) = (script.to_owned(), dispatch.to_owned());
    // The view's current place for the pointer watch, then the interval starts before the receipt is armed.
    vw_platform::native_click::person::refresh(&native.controller());
    person.arm(&interval);
    let native = std::rc::Rc::new(native);
    let view = native.clone();
    crate::frames_win::evaluate(
        &view,
        None,
        &script,
        Box::new(move |answers| {
            if reply.is_closed() {
                return;
            }
            if let Err(reason) = answers
                .and_then(isolated)
                .and_then(|a| vw_preview::validate_frame_point(&request, &a))
            {
                let _ = reply.send(Err(reason));
                return;
            }
            if allowed.as_ref().is_some_and(|check| !check()) {
                let _ = reply.send(Err(
                    "Not run: browser permission changed before native input".into(),
                ));
                return;
            }
            let view = native.clone();
            crate::frames_win::evaluate(
                &view,
                None,
                &dispatch,
                Box::new(move |answers| {
                    if reply.is_closed() {
                        return;
                    }
                    if let Err(reason) = answers
                        .and_then(isolated)
                        .and_then(|a| vw_preview::validate_frame_point(&request, &a))
                    {
                        let _ = reply.send(Err(reason));
                        return;
                    }
                    if allowed.as_ref().is_some_and(|check| !check()) {
                        let _ = reply.send(Err(
                            "Not run: browser permission changed before native input".into(),
                        ));
                        return;
                    }
                    if let Err(reason) = crate::frames_win::act_ready(&native) {
                        let _ = reply.send(Err(reason));
                        return;
                    }
                    let controller = native.controller();
                    // Nothing is sent while the person is pressing, has just pressed or released, or pressed
                    // since arming.
                    if let Err(reason) = person.admit(
                        &interval,
                        vw_platform::native_click::person::holding(&controller),
                        vw_platform::native_click::person::recent(),
                    ) {
                        let _ = reply.send(Err(reason));
                        return;
                    }
                    let _ = reply.send(vw_platform::native_click::windows_click(
                        &controller,
                        point,
                        &sent,
                    ));
                }),
            );
        }),
    );
}
