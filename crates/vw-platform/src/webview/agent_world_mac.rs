//! The agent browser's own script world on macOS (`WKWebView`): script in every frame, cross-site iframes
//! included, without touching the page's own JavaScript.
//!
//! - **World.** On macOS 27+ a configured world (`+[WKContentWorld worldWithConfiguration:]`) that treats closed
//!   shadow roots as open and may create node snapshots and JS handles; the page's own world keeps every default.
//!   On macOS 11-26 a named world (`worldWithName:`) without those extras: closed shadow roots stay closed there.
//! - **Frames.** A user script in that world, in every frame (`forMainFrameOnly:NO`, document start), posts one
//!   message per document to a handler registered for that world only (page scripts cannot reach it). The message's
//!   `WKFrameInfo` is the public target for `evaluateJavaScript:inFrame:inContentWorld:` (macOS 11+), so a
//!   cross-origin frame answers too. A main-frame document forgets every earlier frame (its children went with it).
//! - **Frames by handle (macOS 27+).** `window.webkit.createJSHandle(frames[i])` in the configured world returns a
//!   `WKJSHandle`; `getWindowProxyFrameWithCompletionHandler:` gives that child's `WKFrameInfo` without waiting for
//!   its document-start message.
//!
//! macOS 27 classes and selectors are not in objc2-web-kit 0.3: they are looked up at run time and called with
//! `msg_send!`, so an older macOS simply takes the fallback.
//! Depends on: objc2, objc2-foundation, objc2-web-kit, block2. Called by: vw-webview `mac_agent` (inside Tauri's
//! `with_webview`, on the main thread).
//! Invariants: every function runs on the main thread (the state is thread-local); every `done` is called exactly
//! once; no page-world script is installed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSArray, NSError, NSJSONSerialization, NSObjectProtocol, NSString};
use objc2_web_kit::{
    WKContentWorld, WKFrameInfo, WKScriptMessage, WKScriptMessageHandler, WKUserContentController,
    WKUserScript, WKUserScriptInjectionTime, WKWebView,
};

/// Name of the message handler inside the agent world.
const HANDLER: &str = "vibeWorkFrames";
/// Name of the fallback (macOS 11-26) world.
const WORLD_NAME: &str = "VibeWorkAgent";
/// Most frames remembered per page (a page with more answers from the first ones).
const MAX_FRAMES: usize = 64;

/// The registration script: one message per document, from the agent world only.
const REGISTER: &str = "try{window.webkit.messageHandlers.vibeWorkFrames.postMessage(String(location.href).slice(0,500))}catch(e){}";

/// What the installed world can do on this macOS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldFacts {
    /// A macOS 27 configured world (otherwise a named world, macOS 11-26).
    pub configured: bool,
}

impl WorldFacts {
    /// Closed shadow roots read as open in the agent world.
    #[must_use]
    pub fn closed_shadow_roots(self) -> bool {
        self.configured
    }
    /// `window.webkit.createJSHandle` exists in the agent world.
    #[must_use]
    pub fn js_handles(self) -> bool {
        self.configured
    }
    /// `window.webkit.createNodeSnapshot` exists in the agent world.
    #[must_use]
    pub fn node_snapshots(self) -> bool {
        self.configured
    }
}

/// One frame's answer.
#[derive(Clone, Debug)]
pub struct FrameAnswer {
    /// Registry handle within this main document.
    pub id: String,
    /// Native frame registry generation when this answer was collected.
    pub generation: Option<u64>,
    /// The top document.
    pub main: bool,
    /// `scheme://host:port` of the frame's security origin.
    pub origin: String,
    /// The frame's request URL (as `WKFrameInfo` reports it).
    pub url: String,
    /// The JSON of the script's value, or `WebKit`'s error text.
    pub value: Result<String, String>,
}

/// Receives the answers of every frame.
pub type FramesDone = Box<dyn FnOnce(Result<Vec<FrameAnswer>, String>)>;
/// Receives one evaluation's JSON.
pub type EvalDone = Box<dyn FnOnce(Result<String, String>) + Send>;

/// The per-webview world and its frame registry.
struct Installed {
    world: Retained<WKContentWorld>,
    handler: Retained<FrameHandler>,
    facts: WorldFacts,
}

/// What an all-frames script wrapped by the preview's detached-frame check answers in a frame removed from its page
/// (`vw_preview::DETACHED_FRAME`, the JSON of the string `{"detached":true}`): such a handle leaves the registry.
pub const DETACHED_ANSWER: &str = r#""{\"detached\":true}""#;

thread_local! {
    /// Installed worlds by webview address (main thread only).
    static WORLDS: RefCell<HashMap<usize, Installed>> = RefCell::new(HashMap::new());
}

/// The handler's frames (newest last).
#[derive(Default)]
struct Frames {
    list: RefCell<Vec<(u64, Retained<WKFrameInfo>)>>,
    next: std::cell::Cell<u64>,
    generation: std::cell::Cell<u64>,
}

define_class!(
    /// Remembers the `WKFrameInfo` of every document that ran the agent world's registration script.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VibeWorkAgentFrames"]
    #[ivars = Frames]
    struct FrameHandler;

    unsafe impl NSObjectProtocol for FrameHandler {}

    unsafe impl WKScriptMessageHandler for FrameHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: a valid message for the duration of this main-thread call.
            let frame = unsafe { message.frameInfo() };
            // SAFETY: a plain property read.
            let main = unsafe { frame.isMainFrame() };
            self.ivars()
                .generation
                .set(self.ivars().generation.get().wrapping_add(1));
            let mut list = self.ivars().list.borrow_mut();
            if main {
                list.clear();
            }
            if list.len() >= MAX_FRAMES {
                // Keep the main frame and the newest documents. IDs are never recycled.
                list.remove(1);
            }
            let id = self.ivars().next.get().wrapping_add(1);
            self.ivars().next.set(id);
            list.push((id, frame));
        }
    }
);

impl FrameHandler {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Frames::default());
        // SAFETY: `init` on a freshly allocated NSObject subclass.
        unsafe { msg_send![super(this), init] }
    }
}

/// The macOS 27 configured world, when this macOS has it. `VW_DEV_AGENT_WORLD=named` takes the macOS 11-26 path
/// on a newer macOS (the private engine fixture checks the fallback with it).
fn configured_world() -> Option<Retained<WKContentWorld>> {
    if std::env::var_os("VW_DEV_AGENT_WORLD").is_some_and(|v| v == "named") {
        return None;
    }
    let config_class = AnyClass::get(c"WKContentWorldConfiguration")?;
    let world_class = AnyClass::get(c"WKContentWorld")?;
    // SAFETY: class-object queries; the selector is checked before it is sent.
    let responds: bool =
        unsafe { msg_send![world_class, respondsToSelector: sel!(worldWithConfiguration:)] };
    if !responds {
        return None;
    }
    // SAFETY: `new` on an NSObject subclass; the BOOL setters exist on every WKContentWorldConfiguration (macOS 27
    // SDK header); `worldWithConfiguration:` returns an autoreleased world we retain.
    unsafe {
        let config: Retained<AnyObject> = msg_send![config_class, new];
        let _: () = msg_send![&*config, setAllowAccessingClosedShadowRoots: true];
        let _: () = msg_send![&*config, setJSHandleCreationEnabled: true];
        let _: () = msg_send![&*config, setNodeSnapshotCreationEnabled: true];
        let world: Option<Retained<WKContentWorld>> =
            msg_send![world_class, worldWithConfiguration: &*config];
        world
    }
}

/// Installs the agent world on `webview` (a `WKWebView`) once; later calls answer the same facts.
///
/// # Errors
/// No webview.
pub fn install_agent_world(webview: *mut c_void) -> Result<WorldFacts, String> {
    // SAFETY: Tauri hands over its live `WKWebView` on the main thread inside `with_webview`.
    let Some(view) = (unsafe { webview.cast::<WKWebView>().as_ref() }) else {
        return Err("the preview webview is not available".to_owned());
    };
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("the agent world is installed on the main thread only".to_owned());
    };
    let key = webview as usize;
    if let Some(facts) = WORLDS.with(|w| w.borrow().get(&key).map(|i| i.facts)) {
        return Ok(facts);
    }
    let (world, configured) = match configured_world() {
        Some(world) => (world, true),
        // SAFETY: a class method with a valid string (macOS 11+, the app's minimum).
        None => (
            unsafe { WKContentWorld::worldWithName(&NSString::from_str(WORLD_NAME), mtm) },
            false,
        ),
    };
    let handler = FrameHandler::new(mtm);
    // SAFETY: the webview's own user content controller on the main thread; WebKit retains the script and the handler
    // (which holds no reference back to the webview).
    unsafe {
        let controller = view.configuration().userContentController();
        controller.addScriptMessageHandler_contentWorld_name(
            ProtocolObject::from_ref(&*handler),
            &world,
            &NSString::from_str(HANDLER),
        );
        let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            WKUserScript::alloc(mtm),
            &NSString::from_str(REGISTER),
            WKUserScriptInjectionTime::AtDocumentStart,
            false,
            &world,
        );
        controller.addUserScript(&script);
    }
    let facts = WorldFacts { configured };
    WORLDS.with(|w| {
        w.borrow_mut().insert(
            key,
            Installed {
                world,
                handler,
                facts,
            },
        )
    });
    Ok(facts)
}

/// Forgets a closed webview's world (`WebKit` releases the scripts with the webview).
pub fn forget_agent_world(webview: *mut c_void) {
    WORLDS.with(|w| w.borrow_mut().remove(&(webview as usize)));
}

/// The JSON of an evaluation result, or the error's text.
///
/// # Safety
/// `value` and `error` are the completion handler's arguments (valid or null) on the main thread.
unsafe fn answer(value: *mut AnyObject, error: *mut NSError) -> Result<String, String> {
    // SAFETY: valid or null per the caller.
    if let Some(error) = unsafe { error.as_ref() } {
        return Err(error.localizedDescription().to_string());
    }
    // SAFETY: valid or null per the caller.
    let Some(value) = (unsafe { value.as_ref() }) else {
        return Ok("null".to_owned());
    };
    // SAFETY: fragments are allowed, so any JSON-compatible value serialises; WKJSHandle and other objects fail
    // into the error arm.
    unsafe {
        NSJSONSerialization::dataWithJSONObject_options_error(
            value,
            objc2_foundation::NSJSONWritingOptions::FragmentsAllowed,
        )
    }
    .map(|data| String::from_utf8_lossy(&data.to_vec()).into_owned())
    .map_err(|_| {
        format!(
            "a value JSON cannot hold ({})",
            value.class().name().to_string_lossy()
        )
    })
}

fn frame_facts(frame: &WKFrameInfo) -> (bool, String, String) {
    // SAFETY: property reads of a WKFrameInfo (a data-only object) on the main thread.
    unsafe {
        let origin = frame.securityOrigin();
        let port = origin.port();
        let origin = if port > 0 {
            format!("{}://{}:{port}", origin.protocol(), origin.host())
        } else {
            format!("{}://{}", origin.protocol(), origin.host())
        };
        let url = frame
            .request()
            .URL()
            .and_then(|u| u.absoluteString())
            .map(|s| s.to_string())
            .unwrap_or_default();
        (frame.isMainFrame(), origin, url)
    }
}

/// Collects one answer per frame and calls `done` after the last.
struct Gather {
    done: Option<FramesDone>,
    answers: Vec<FrameAnswer>,
    left: usize,
}

type Shared = std::rc::Rc<RefCell<Gather>>;

fn gather(done: FramesDone, left: usize) -> Shared {
    std::rc::Rc::new(RefCell::new(Gather {
        done: Some(done),
        answers: Vec::new(),
        left,
    }))
}

fn push(state: &Shared, answer: FrameAnswer) {
    let mut s = state.borrow_mut();
    s.answers.push(answer);
    s.left = s.left.saturating_sub(1);
    if s.left == 0
        && let Some(done) = s.done.take()
    {
        let answers = std::mem::take(&mut s.answers);
        drop(s);
        done(Ok(answers));
    }
}

fn registry_generation(webview: *mut c_void) -> Option<u64> {
    WORLDS.with(|worlds| {
        worlds
            .borrow()
            .get(&(webview as usize))
            .map(|installed| installed.handler.ivars().generation.get())
    })
}

/// Evaluates `source` in `frame` of `webview` in `world` and pushes the answer.
fn eval_in_frame(
    id: String,
    webview: *mut c_void,
    frame: &WKFrameInfo,
    world: &WKContentWorld,
    source: &NSString,
    state: &Shared,
) {
    let (main, origin, url) = frame_facts(frame);
    // SAFETY: the webview outlives this main-thread chain (the caller holds it while it waits).
    let Some(view) = (unsafe { webview.cast::<WKWebView>().as_ref() }) else {
        push(
            state,
            FrameAnswer {
                id: id.clone(),
                generation: registry_generation(webview),
                main,
                origin,
                url,
                value: Err("the preview webview is not available".to_owned()),
            },
        );
        return;
    };
    let state = state.clone();
    let block = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
        // SAFETY: WebKit's completion arguments, on the main thread.
        let gone = unsafe { error.as_ref() }.is_some_and(|error| {
            error.domain().to_string() == "WKErrorDomain" && error.code() == 12
        });
        // SAFETY: WebKit's completion arguments, on the main thread.
        let value = unsafe { answer(value, error) };
        // A frame removed from its page whose retained handle still answers from the detached document leaves the
        // registry now, the same way as a frame WebKit reports gone: its later removal can then not change the
        // registry during a checked click.
        if gone || value.as_deref() == Ok(DETACHED_ANSWER) {
            WORLDS.with(|worlds| {
                if let Some(installed) = worlds.borrow().get(&(webview as usize)) {
                    let frames = installed.handler.ivars();
                    let mut list = frames.list.borrow_mut();
                    let before = list.len();
                    list.retain(|(key, _)| key.to_string() != id);
                    if before != list.len() {
                        frames
                            .generation
                            .set(frames.generation.get().wrapping_add(1));
                    }
                }
            });
        }
        push(
            &state,
            FrameAnswer {
                id: id.clone(),
                generation: registry_generation(webview),
                main,
                origin: origin.clone(),
                url: url.clone(),
                value,
            },
        );
    });
    // SAFETY: a live webview, a frame info WebKit gave us and our world; WebKit calls the block once (an invalid
    // frame answers WKErrorJavaScriptInvalidFrameTarget).
    unsafe {
        view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            source,
            Some(frame),
            world,
            Some(&block),
        );
    }
}

/// Evaluates `script` in the agent world of every frame registered for the current document.
pub fn evaluate_in_frames(webview: *mut c_void, script: &str, done: FramesDone) {
    let key = webview as usize;
    let Some((world, frames)) = WORLDS.with(|w| {
        w.borrow()
            .get(&key)
            .map(|i| (i.world.clone(), i.handler.ivars().list.borrow().clone()))
    }) else {
        done(Err("the agent world is not installed".to_owned()));
        return;
    };
    if frames.is_empty() {
        done(Ok(Vec::new()));
        return;
    }
    let state = gather(done, frames.len());
    let source = NSString::from_str(script);
    for (id, frame) in frames {
        eval_in_frame(id.to_string(), webview, &frame, &world, &source, &state);
    }
}

/// Evaluates only the registered frame handle; invalid handles fail without main-frame fallback.
pub fn evaluate_registered(webview: *mut c_void, id: &str, script: &str, done: FramesDone) {
    let selected = id.parse::<u64>().ok().and_then(|wanted| {
        WORLDS.with(|worlds| {
            let worlds = worlds.borrow();
            let installed = worlds.get(&(webview as usize))?;
            let frame = installed
                .handler
                .ivars()
                .list
                .borrow()
                .iter()
                .find(|(id, _)| *id == wanted)?
                .1
                .clone();
            Some((installed.world.clone(), frame))
        })
    });
    let Some((world, frame)) = selected else {
        done(Err("frame is stale or detached".into()));
        return;
    };
    eval_in_frame(
        id.to_owned(),
        webview,
        &frame,
        &world,
        &NSString::from_str(script),
        &gather(done, 1),
    );
}

/// Evaluates `script` in the main frame, in the agent world or (`page_world`) the page's own world.
pub fn evaluate_main(webview: *mut c_void, script: &str, page_world: bool, done: EvalDone) {
    let key = webview as usize;
    let world = if page_world {
        // SAFETY: a class property on the main thread.
        MainThreadMarker::new().map(|mtm| unsafe { WKContentWorld::pageWorld(mtm) })
    } else {
        WORLDS.with(|w| w.borrow().get(&key).map(|i| i.world.clone()))
    };
    let Some(world) = world else {
        done(Err("the agent world is not installed".to_owned()));
        return;
    };
    // SAFETY: as in `install_agent_world`.
    let Some(view) = (unsafe { webview.cast::<WKWebView>().as_ref() }) else {
        done(Err("the preview webview is not available".to_owned()));
        return;
    };
    let done = RefCell::new(Some(done));
    let block = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
        if let Some(done) = done.borrow_mut().take() {
            // SAFETY: WebKit's completion arguments, on the main thread.
            done(unsafe { answer(value, error) });
        }
    });
    // SAFETY: a live webview and world; WebKit calls the block once.
    unsafe {
        view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(script),
            None,
            &world,
            Some(&block),
        );
    }
}

/// macOS 27: evaluates `script` in each direct child frame of the main frame, found through JS handles of
/// `window.frames` (`WKJSHandle getWindowProxyFrameWithCompletionHandler:`), in the agent world.
pub fn evaluate_in_child_frames_by_handle(webview: *mut c_void, script: &str, done: FramesDone) {
    let key = webview as usize;
    let Some((world, facts)) =
        WORLDS.with(|w| w.borrow().get(&key).map(|i| (i.world.clone(), i.facts)))
    else {
        done(Err("the agent world is not installed".to_owned()));
        return;
    };
    if !facts.js_handles() {
        done(Err("JS handles need macOS 27".to_owned()));
        return;
    }
    // SAFETY: as in `install_agent_world`.
    let Some(view) = (unsafe { webview.cast::<WKWebView>().as_ref() }) else {
        done(Err("the preview webview is not available".to_owned()));
        return;
    };
    let source = NSString::from_str(script);
    let frames_world = world.clone();
    let done = RefCell::new(Some(done));
    let block = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
        let Some(done) = done.borrow_mut().take() else {
            return;
        };
        // SAFETY: WebKit's completion arguments, on the main thread.
        if let Some(error) = unsafe { error.as_ref() } {
            done(Err(error.localizedDescription().to_string()));
            return;
        }
        // SAFETY: valid or null; an NSArray of WKJSHandle when the script returned handles.
        let Some(array) = (unsafe { value.as_ref() }).and_then(|v| v.downcast_ref::<NSArray>())
        else {
            done(Ok(Vec::new()));
            return;
        };
        let handles: Vec<Retained<AnyObject>> = array.iter().collect();
        if handles.is_empty() {
            done(Ok(Vec::new()));
            return;
        }
        let state = gather(done, handles.len());
        for handle in handles {
            let (state, world, source) = (state.clone(), frames_world.clone(), source.clone());
            let frame_block = RcBlock::new(move |frame: *mut WKFrameInfo| {
                // SAFETY: valid or null on the main thread.
                match unsafe { frame.as_ref() } {
                    Some(frame) => {
                        eval_in_frame(String::new(), webview, frame, &world, &source, &state);
                    }
                    None => push(
                        &state,
                        FrameAnswer {
                            id: String::new(),
                            generation: registry_generation(webview),
                            main: false,
                            origin: String::new(),
                            url: String::new(),
                            value: Err("the handle named no frame".to_owned()),
                        },
                    ),
                }
            });
            // SAFETY: `handle` is a WKJSHandle (macOS 27); the selector is checked before it is sent.
            unsafe {
                let responds: bool = msg_send![
                    &*handle,
                    respondsToSelector: sel!(getWindowProxyFrameWithCompletionHandler:)
                ];
                if responds {
                    let _: () = msg_send![
                        &*handle,
                        getWindowProxyFrameWithCompletionHandler: &*frame_block
                    ];
                } else {
                    frame_block.call((std::ptr::null_mut(),));
                }
            }
        }
    });
    let collect = "Array.from({length: window.frames.length}, (_, i) => window.webkit.createJSHandle(window.frames[i]))";
    // SAFETY: a live webview and the configured world; WebKit calls the block once.
    unsafe {
        view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(collect),
            None,
            &world,
            Some(&block),
        );
    }
}
