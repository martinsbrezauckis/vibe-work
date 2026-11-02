//! `WebView2` frame APIs for browser scripts, including nested Frame7 children.
//! Called on the UI thread by vw-webview; handles expire on native destruction, scripts bind document identity.
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use webview2_com::{
    ExecuteScriptCompletedHandler, FrameChildFrameCreatedEventHandler, FrameCreatedEventHandler,
    FrameDestroyedEventHandler, FrameNavigationStartingEventHandler,
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_4, ICoreWebView2Controller, ICoreWebView2Frame, ICoreWebView2Frame2,
        ICoreWebView2Frame7,
    },
    take_pwstr,
};
use windows::core::{HSTRING, Interface};

/// A child frame's answer when it could not be evaluated: a JSON string the browser's frame check reads as unreadable.
const UNREADABLE: &str = r#""{\"unreadable\":true}""#;

/// Native handle, main-frame flag and JSON result.
pub type Answer = (String, bool, String, String, String);
/// Completes a bounded host request.
pub type Done = Box<dyn FnOnce(Result<Vec<Answer>, String>)>;
#[derive(Default)]
struct Frames {
    next: u64,
    list: HashMap<String, (ICoreWebView2Frame, String)>,
}
thread_local! { static VIEWS: RefCell<HashMap<usize, Rc<RefCell<Frames>>>> = RefCell::new(HashMap::new()); }

fn register(state: &Rc<RefCell<Frames>>, frame: &ICoreWebView2Frame) -> windows::core::Result<()> {
    let id = {
        let mut state = state.borrow_mut();
        if state.list.len() >= 63 {
            return Ok(());
        }
        state.next += 1;
        let id = state.next.to_string();
        state
            .list
            .insert(id.clone(), (frame.clone(), "about:blank".into()));
        id
    };
    let current = Rc::new(RefCell::new(id));
    let destroyed_id = current.clone();
    let weak = Rc::downgrade(state);
    let destroyed = FrameDestroyedEventHandler::create(Box::new(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.borrow_mut().list.remove(&*destroyed_id.borrow());
        }
        Ok(())
    }));
    let weak = Rc::downgrade(state);
    let child = FrameChildFrameCreatedEventHandler::create(Box::new(move |_, args| {
        if let (Some(state), Some(args)) = (weak.upgrade(), args) {
            // SAFETY: WebView2 supplies the event's live frame on the owning UI thread.
            register(&state, &unsafe { args.Frame()? })?;
        }
        Ok(())
    }));
    let weak = Rc::downgrade(state);
    let navigating = FrameNavigationStartingEventHandler::create(Box::new(move |frame, args| {
        if let (Some(state), Some(frame)) = (weak.upgrade(), frame) {
            let mut state = state.borrow_mut();
            state.list.remove(&*current.borrow());
            state.next += 1;
            let id = state.next.to_string();
            current.borrow_mut().clone_from(&id);
            let mut uri = windows::core::PWSTR::null();
            // SAFETY: the navigation event owns this URI, independently of the page's script world.
            let address = if let Some(args) = args {
                unsafe {
                    args.Uri(&raw mut uri)?;
                }
                take_pwstr(uri)
            } else {
                String::new()
            };
            state.list.insert(id, (frame, address));
        }
        Ok(())
    }));
    // SAFETY: frame and handlers are UI-thread COM objects; `WebView2` retains the event handlers.
    unsafe {
        let mut token = 0;
        frame.add_Destroyed(&destroyed, &raw mut token)?;
        frame
            .cast::<ICoreWebView2Frame2>()?
            .add_NavigationStarting(&navigating, &raw mut token)?;
        // Nested frame API is required; failure leaves nested frames explicitly unavailable.
        if let Ok(nested) = frame.cast::<ICoreWebView2Frame7>() {
            nested.add_FrameCreated(&child, &raw mut token)?;
        }
    }
    Ok(())
}

/// Registers frame creation before navigation.
/// # Errors
/// Missing `WebView2` frame API or COM failure.
pub fn install(controller: &ICoreWebView2Controller) -> Result<(), String> {
    let key = controller.as_raw() as usize;
    let state = Rc::new(RefCell::new(Frames::default()));
    let callback_state = state.clone();
    let handler = FrameCreatedEventHandler::create(Box::new(move |_, args| {
        if let Some(args) = args {
            // SAFETY: callback argument owned by WebView2 for this UI-thread event.
            register(&callback_state, &unsafe { args.Frame()? })?;
        }
        Ok(())
    }));
    // SAFETY: called only with a live controller on its UI thread.
    unsafe {
        let view: ICoreWebView2_4 = controller
            .CoreWebView2()
            .and_then(|v| v.cast())
            .map_err(|e| e.to_string())?;
        let mut token = 0;
        view.add_FrameCreated(&handler, &raw mut token)
            .map_err(|e| e.to_string())?;
    }
    VIEWS.with(|views| views.borrow_mut().insert(key, state));
    Ok(())
}

/// Drops host frame handles when a webview closes.
pub fn forget(controller: &ICoreWebView2Controller) {
    VIEWS.with(|views| views.borrow_mut().remove(&(controller.as_raw() as usize)));
}

struct Pending {
    left: usize,
    answers: Vec<Answer>,
    error: bool,
    done: Option<Done>,
}
fn finish(state: &Rc<RefCell<Pending>>, answer: Result<Answer, String>) {
    let mut state = state.borrow_mut();
    match answer {
        Ok(answer) => state.answers.push(answer),
        Err(_) => state.error = true,
    }
    state.left = state.left.saturating_sub(1);
    if state.left == 0
        && let Some(done) = state.done.take()
    {
        let answer = if state.error {
            Err("frame document changed, detached, or could not be evaluated".into())
        } else {
            Ok(std::mem::take(&mut state.answers))
        };
        drop(state);
        done(answer);
    }
}

/// Evaluates in all live frames or one exact native handle, with no main-frame fallback.
pub fn evaluate(
    controller: &ICoreWebView2Controller,
    target: Option<&str>,
    script: &str,
    done: Done,
) {
    let frames = VIEWS.with(|views| {
        views
            .borrow()
            .get(&(controller.as_raw() as usize))
            .map(|s| s.borrow().list.clone())
    });
    let Some(frames) = frames else {
        done(Err("frame registry is unavailable".into()));
        return;
    };
    let mut selected: Vec<_> = frames
        .into_iter()
        .filter(|(id, _)| target.is_none_or(|t| t == id))
        .map(|(id, (f, url))| (id, Some(f), url))
        .collect();
    if target.is_none_or(|t| t == "main") {
        let mut uri = windows::core::PWSTR::null();
        // SAFETY: the controller's native Source getter returns a COM-owned string, freed below.
        if let Err(error) = unsafe {
            controller
                .CoreWebView2()
                .and_then(|v| v.Source(&raw mut uri))
        } {
            done(Err(error.to_string()));
            return;
        }
        selected.push(("main".into(), None, take_pwstr(uri)));
    }
    if selected.is_empty() {
        done(Err("frame is stale or detached".into()));
        return;
    }
    let state = Rc::new(RefCell::new(Pending {
        left: selected.len(),
        answers: Vec::new(),
        error: false,
        done: Some(done),
    }));
    // Native serialization of a null-prototype envelope cannot call page-owned toJSON hooks.
    let script = format!(
        "(() => ({{__proto__:null, origin:window.__vwNativeFrameOrigin, value:({})}}))()",
        script.trim().trim_end_matches(';')
    );
    // In an evaluation of every frame, a child frame that cannot run the script (a sandbox without scripts, a frame
    // destroyed meanwhile) answers as unreadable, so the caller leaves that frame out instead of failing them all.
    let every = target.is_none();
    for (id, frame, url) in selected {
        let main = frame.is_none();
        let pending = state.clone();
        let (child_id, child_url) = (id.clone(), url.clone());
        let unreadable = move |answer: Result<Answer, String>| match answer {
            Err(_) if every && !main => {
                Ok((child_id, false, child_url, String::new(), UNREADABLE.into()))
            }
            other => other,
        };
        let failed = unreadable.clone();
        let handler = ExecuteScriptCompletedHandler::create(Box::new(move |status, value| {
            finish(
                &pending,
                unreadable(status.map_err(|e| e.to_string()).and_then(|()| {
                    let envelope: serde_json::Value =
                        serde_json::from_str(&value).map_err(|_| "invalid frame envelope")?;
                    let origin = envelope["origin"]
                        .as_str()
                        .ok_or("frame origin is unavailable")?
                        .to_owned();
                    let value = envelope
                        .get("value")
                        .ok_or("frame result is unavailable")?
                        .to_string();
                    Ok((id, main, url, origin, value))
                })),
            );
            Ok(())
        }));
        // SAFETY: all handles and completion handlers stay on the UI thread. `WebView2` retains the callback.
        let result = unsafe {
            match frame {
                Some(frame) => frame
                    .cast::<ICoreWebView2Frame2>()
                    .and_then(|f| f.ExecuteScript(&HSTRING::from(&script), &handler)),
                None => controller
                    .CoreWebView2()
                    .and_then(|v| v.ExecuteScript(&HSTRING::from(&script), &handler)),
            }
        };
        if let Err(error) = result {
            finish(&state, failed(Err(error.to_string())));
        }
    }
}

/// Installs the immutable origin stamp before the first admitted navigation. Completion means it is registered.
pub fn install_origin(
    controller: &ICoreWebView2Controller,
    done: Box<dyn FnOnce(Result<(), String>)>,
) {
    let done = Rc::new(RefCell::new(Some(done)));
    let pending = done.clone();
    let handler = webview2_com::AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(
        Box::new(move |status, _| {
            if let Some(done) = pending.borrow_mut().take() {
                done(status.map_err(|e| e.to_string()));
            }
            Ok(())
        }),
    );
    // SAFETY: the controller is live on its UI thread; WebView2 retains the completion handler.
    let result = unsafe {
        controller.CoreWebView2().and_then(|v| {
            v.AddScriptToExecuteOnDocumentCreated(
                &HSTRING::from(include_str!("frame_origin.js")),
                &handler,
            )
        })
    };
    if let Err(error) = result
        && let Some(done) = done.borrow_mut().take()
    {
        done(Err(error.to_string()));
    }
}
