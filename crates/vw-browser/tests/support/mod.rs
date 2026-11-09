//! Real page fixture for browser regressions, driven through `PreviewService`.
//! Depends on Node, playwright-core and its installed Chromium; called by browser integration tests.
//! Each test owns its subprocess, storage and page; page scripts are the production scripts.
#![allow(dead_code)]
mod faults;
mod pointer;
mod readiness;
mod screenshot;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};
use url::Url;
use vw_browser::capture::{Fetch, FetchFuture};
use vw_browser::{Browser, Sessions};
mod cdp;
pub use cdp::NATIVE_CLICK_WITHOUT_WINDOW;
use cdp::{CdpRequest, PICKERS, cdp_frames, cdp_worker};
use faults::take_one;
use vw_preview::{
    ConsoleEntry, EngineError, EngineFuture, HistoryStep, MountId, MountSpec, OpenRequest,
    PageEvent, Placement, PreviewConfig, PreviewEngine, PreviewHooks, PreviewService, Rect,
    Storage,
};
pub const SESSION: &str = "fixture";
pub const URL: &str = "https://93.184.215.14/";
/// How long after the person's last press or release this fake engine still refuses checked input.
pub const PERSON_RECENT: Duration = Duration::from_millis(300);
pub(crate) struct Pipe {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Drop for Pipe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn rpc_blocking(pipe: &Mutex<Pipe>, request: &Value) -> Result<Value, EngineError> {
    let mut pipe = pipe.lock().unwrap();
    writeln!(pipe.input, "{request}").map_err(|e| EngineError::Platform(e.to_string()))?;
    pipe.input
        .flush()
        .map_err(|e| EngineError::Platform(e.to_string()))?;
    let mut line = String::new();
    pipe.output
        .read_line(&mut line)
        .map_err(|e| EngineError::Platform(e.to_string()))?;
    let reply: Value =
        serde_json::from_str(&line).map_err(|e| EngineError::Platform(e.to_string()))?;
    if let Some(error) = reply["error"].as_str() {
        return Err(EngineError::Platform(error.to_owned()));
    }
    Ok(reply["value"].clone())
}

/// The frame registry a lagging host still reports: (frame id, generation).
type Lagged = Vec<(String, Option<u64>)>;

pub struct PageEngine {
    pipe: Arc<Mutex<Pipe>>,
    cdp: Mutex<Option<std::sync::mpsc::Sender<CdpRequest>>>,
    hooks: OnceLock<Arc<dyn PreviewHooks>>,
    pub hold_pin: AtomicBool,
    pub fail_pin: AtomicBool,
    pub navigate_during_shot: AtomicBool,
    pub pinned: Notify,
    pub release: Semaphore,
    pub on_pin: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    pub on_click: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    pub native_clicks: AtomicU64,
    pub focus_required: AtomicBool,
    pub input_markers: Mutex<Vec<vw_preview::action_tracking::ActionMarker>>,
    pub reverse_frames: AtomicBool,
    pub after_verify: Mutex<Option<String>>,
    pub after_point_validation: Mutex<Option<String>>,
    pub user_click_before_dispatch: AtomicBool,
    pub user_click_after_mark: AtomicBool,
    pub after_receipt_poll: Mutex<Option<String>>,
    pub user_click_after_dispatch: AtomicBool,
    pub user_click_after_poll: Mutex<Option<(f64, f64)>>,
    pub page_world: AtomicBool,
    pub isolate_scripts: AtomicBool,
    pub unreadable_receipt: AtomicBool,
    /// Observations see every child frame without a native address, as a frame not yet committed to a document.
    pub unaddressed_children: AtomicBool,
    /// The next this many observations see the main frame without a native address.
    pub unaddressed_main: AtomicU64,
    /// The person's own click (a real engine observes it on the native view) after the dispatch mark.
    pub person_click_after_mark: AtomicBool,
    /// The person's own click after the agent's input was submitted, before its receipt is collected.
    pub person_click_after_dispatch: AtomicBool,
    /// The agent's input is submitted but never reaches the page as DOM events.
    pub agent_input_lost: AtomicBool,
    /// The host has not handled frame registrations made after the click's registry snapshot yet.
    pub registry_lag: AtomicBool,
    lagged: Mutex<Option<Lagged>>,
    /// The person holds a pointer button down on the view right now.
    pub person_holding: AtomicBool,
    /// This engine's host-side watch of the person's pointer: only `person_click` feeds it, never page script
    /// and never the agent's own input.
    pub person: Arc<vw_preview::PersonPointer>,
    /// The next this many observations get an empty answer from every child frame, as from a document being replaced.
    pub empty_children: AtomicU64,
    /// The next this many observations fail as the engines fail one whose frame was removed during the evaluation.
    pub detached_during_read: AtomicU64,
    /// Observations name the main frame's native address `about:srcdoc`.
    pub main_srcdoc: AtomicBool,
    /// Observations answer every child frame twice more, as `WebKit` on macOS does through old handles of the same
    /// frame: once with the same native address, once with the address the frame had before.
    pub duplicate_children: AtomicBool,
    /// Observations get no answer at all from inline child frames, as a Linux frame registry that never saw them.
    pub silent_inline_children: AtomicBool,
    /// Observations name the main frame's native address so.
    pub main_address: Mutex<Option<String>>,
    /// Observations name a `data:` frame's native address percent-encoded, as `WebKit` on macOS does.
    pub encoded_data_addresses: AtomicBool,
    /// Every all-frames evaluation answers each child frame as one that could not be evaluated (as the engines
    /// answer a frame whose script cannot run, a scriptless sandbox on Windows for one).
    pub unreadable_children: AtomicBool,
    /// Only whole-page reads (observe) answer each child frame as unreadable; other all-frames evaluations (the
    /// checked click's frame registry and receipt) answer normally, as `WebKit` on macOS can.
    pub unreadable_children_in_reads: AtomicBool,
    /// All-frames evaluations that also get a removed child frame's answer (`faults.rs`).
    pub detached_child_evaluations: AtomicU64,
}

impl PageEngine {
    fn new() -> Arc<Self> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut command = Command::new("node");
        command
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/page.mjs"
            ))
            .env("VW_BROWSER_UI_PACKAGE", root.join("ui/package.json"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if std::env::var_os("PLAYWRIGHT_BROWSERS_PATH").is_none() {
            command.env("PLAYWRIGHT_BROWSERS_PATH", root.join("target/playwright"));
        }
        let mut child = command
            .spawn()
            .unwrap_or_else(|error| panic!("could not spawn Node browser fixture: {error}"));
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Arc::new(Self {
            pipe: Arc::new(Mutex::new(Pipe {
                child,
                input,
                output,
            })),
            cdp: Mutex::new(None),
            hooks: OnceLock::new(),
            hold_pin: AtomicBool::new(false),
            fail_pin: AtomicBool::new(false),
            navigate_during_shot: AtomicBool::new(false),
            pinned: Notify::new(),
            release: Semaphore::new(0),
            on_pin: Mutex::new(None),
            on_click: Mutex::new(None),
            native_clicks: AtomicU64::new(0),
            focus_required: AtomicBool::new(false),
            input_markers: Mutex::default(),
            reverse_frames: AtomicBool::new(false),
            after_verify: Mutex::new(None),
            after_point_validation: Mutex::new(None),
            user_click_before_dispatch: AtomicBool::new(false),
            user_click_after_mark: AtomicBool::new(false),
            after_receipt_poll: Mutex::new(None),
            user_click_after_dispatch: AtomicBool::new(false),
            user_click_after_poll: Mutex::new(None),
            page_world: AtomicBool::new(false),
            isolate_scripts: AtomicBool::new(false),
            unreadable_receipt: AtomicBool::new(false),
            unaddressed_children: AtomicBool::new(false),
            unaddressed_main: AtomicU64::new(0),
            person_click_after_mark: AtomicBool::new(false),
            person_click_after_dispatch: AtomicBool::new(false),
            agent_input_lost: AtomicBool::new(false),
            registry_lag: AtomicBool::new(false),
            lagged: Mutex::new(None),
            person_holding: AtomicBool::new(false),
            person: Arc::default(),
            empty_children: AtomicU64::new(0),
            detached_during_read: AtomicU64::new(0),
            main_srcdoc: AtomicBool::new(false),
            duplicate_children: AtomicBool::new(false),
            silent_inline_children: AtomicBool::new(false),
            main_address: Mutex::new(None),
            encoded_data_addresses: AtomicBool::new(false),
            unreadable_children: AtomicBool::new(false),
            unreadable_children_in_reads: AtomicBool::new(false),
            detached_child_evaluations: AtomicU64::new(0),
        })
    }

    async fn rpc(&self, request: Value) -> Result<Value, EngineError> {
        let pipe = self.pipe.clone();
        tokio::task::spawn_blocking(move || rpc_blocking(&pipe, &request))
            .await
            .map_err(|e| EngineError::Platform(e.to_string()))?
    }

    async fn cdp_frames(
        &self,
        frame: Option<String>,
        script: String,
        input: bool,
    ) -> Result<Vec<vw_preview::FrameAnswer>, EngineError> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.cdp
            .lock()
            .unwrap()
            .get_or_insert_with(|| cdp_worker(self.pipe.clone()))
            .send(CdpRequest::Evaluate(frame, script, reply, input))
            .map_err(|e| EngineError::Platform(e.to_string()))?;
        answer
            .await
            .map_err(|e| EngineError::Platform(e.to_string()))?
            .map_err(EngineError::Platform)
    }

    pub async fn eval(&self, script: &str) -> Value {
        self.rpc(json!({"op":"evaluate", "script":script}))
            .await
            .unwrap()
    }

    /// Opt-in public-page benchmarks remove the offline fixture route in this disposable browser.
    pub async fn public_pages(&self) {
        self.rpc(json!({"op":"publicPages"})).await.unwrap();
    }

    /// Runs `script` as page script in every frame's own world (cross-origin frames included).
    pub async fn eval_in_frame_pages(&self, script: &str) {
        self.rpc(json!({"op":"framesPageWorld", "script":script}))
            .await
            .unwrap();
    }

    /// The person clicks the page with the real pointer (Chromium input, outside page script).
    pub async fn person_click(&self, x: f64, y: f64) {
        // A real engine sees the press and the release on the native view before the page does.
        self.person.observed();
        self.person.observed();
        self.rpc(json!({"op":"click", "x":x, "y":y})).await.unwrap();
    }

    pub fn hooks(&self) -> Arc<dyn PreviewHooks> {
        self.hooks.get().unwrap().clone()
    }

    pub async fn snapshot(&self) -> Value {
        self.eval("(() => { const nodes = [document.querySelector('#styled'), document.querySelector('#bare'), document.querySelector('#shadow').shadowRoot.querySelector('#inner')]; return { x:scrollX, y:scrollY, styles:nodes.map(e => e.getAttribute('style')), pins:nodes.map(e => e.hasAttribute('data-vw-capture-pin')) }; })()").await
    }

    pub async fn viewport(&self, width: u32, height: u32) {
        self.rpc(json!({"op":"viewport","width":width,"height":height}))
            .await
            .unwrap();
    }

    /// Evaluates through the chosen host transport before applying the test's fault switches.
    async fn frame_answers(
        &self,
        frame: Option<String>,
        script: &str,
        input: bool,
    ) -> Result<Vec<vw_preview::FrameAnswer>, EngineError> {
        let isolated = self.isolate_scripts.load(Ordering::Relaxed);
        // The DevTools registry is isolated by construction; page-world fixtures keep their own path.
        let answers = if cdp_frames() && !self.page_world.load(Ordering::Relaxed) {
            self.cdp_frames(frame, script.to_owned(), input).await?
        } else {
            let value = self
                .rpc(json!({"op":"frames", "frame":frame, "script":script, "isolated":isolated}))
                .await?;
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|v| vw_preview::FrameAnswer {
                    id: v["id"].as_str().unwrap().into(),
                    generation: v["generation"].as_u64(),
                    main: v["main"] == true,
                    url: v["url"].as_str().unwrap().into(),
                    origin: v["origin"].as_str().map(str::to_owned),
                    value: v["value"].as_str().unwrap().into(),
                })
                .collect::<Vec<_>>()
        };
        Ok(answers)
    }

    /// The fault switches a test set for a whole-page read (observe), applied to the engine's frame answers.
    fn observe_faults(
        &self,
        whole_page: bool,
        script: &str,
        answers: &mut Vec<vw_preview::FrameAnswer>,
    ) -> Result<(), EngineError> {
        let observe = whole_page && script.contains("\"observe\":true");
        if observe && take_one(&self.detached_during_read) {
            return Err(EngineError::Platform(
                "frame document changed, detached, or could not be evaluated".into(),
            ));
        }
        if observe && take_one(&self.empty_children) {
            for answer in answers.iter_mut().filter(|a| !a.main) {
                answer.value = "\"\"".into();
            }
        }
        if observe && self.main_srcdoc.load(Ordering::Relaxed) {
            for answer in answers.iter_mut().filter(|a| a.main) {
                "about:srcdoc".clone_into(&mut answer.url);
            }
        }
        if observe && let Some(address) = self.main_address.lock().unwrap().clone() {
            for answer in answers.iter_mut().filter(|a| a.main) {
                answer.url.clone_from(&address);
            }
        }
        if observe && self.unreadable_children_in_reads.load(Ordering::Relaxed) {
            for answer in answers.iter_mut().filter(|a| !a.main) {
                vw_preview::UNREADABLE_FRAME.clone_into(&mut answer.value);
            }
        }
        if whole_page {
            self.add_detached_child(answers);
        }
        if whole_page && self.unreadable_children.load(Ordering::Relaxed) {
            for answer in answers.iter_mut().filter(|a| !a.main) {
                vw_preview::UNREADABLE_FRAME.clone_into(&mut answer.value);
            }
        }
        if observe && self.encoded_data_addresses.load(Ordering::Relaxed) {
            for answer in answers.iter_mut().filter(|a| a.url.starts_with("data:")) {
                answer.url = answer
                    .url
                    .replace('<', "%3C")
                    .replace('>', "%3E")
                    .replace(' ', "%20");
            }
        }
        if observe && self.silent_inline_children.load(Ordering::Relaxed) {
            answers.retain(|a| a.main || a.url != "about:srcdoc");
        }
        if observe && self.duplicate_children.load(Ordering::Relaxed) {
            let children: Vec<_> = answers.iter().filter(|a| !a.main).cloned().collect();
            for child in children {
                let mut same = child.clone();
                same.id = format!("{}-same", child.id);
                let mut old = child;
                old.id = format!("{}-old", old.id);
                "https://old.example.test/".clone_into(&mut old.url);
                answers.push(same);
                answers.push(old);
            }
        }
        if observe {
            let left = self.unaddressed_main.load(Ordering::Relaxed);
            let main = left > 0
                && self
                    .unaddressed_main
                    .compare_exchange(left, left - 1, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok();
            for answer in answers.iter_mut() {
                if (answer.main && main)
                    || (!answer.main && self.unaddressed_children.load(Ordering::Relaxed))
                {
                    answer.url.clear();
                }
            }
        }
        Ok(())
    }

    async fn load(&self, id: &MountId, url: Url, init: Option<String>) -> Result<(), EngineError> {
        let hooks = self.hooks.get().unwrap();
        assert!(hooks.allow_navigation(&id.label, &url));
        hooks.page_event(&id.label, PageEvent::Started(url.clone()));
        self.rpc(json!({"op":if init.is_some() { "mount" } else { "navigate" }, "url":url.as_str(), "init":init})).await?;
        hooks.page_event(&id.label, PageEvent::Finished(url));
        Ok(())
    }
}

impl PreviewEngine for PageEngine {
    #[cfg(feature = "click-marker")]
    fn show_input_marker(
        &self,
        _: &MountId,
        point: vw_preview::action_tracking::ActionMarker,
    ) -> EngineFuture<'_, ()> {
        self.input_markers.lock().unwrap().push(point);
        Box::pin(async { Ok(()) })
    }
    fn isolated_frame_input(&self) -> bool {
        !self.page_world.load(Ordering::Relaxed)
    }
    fn evaluate_frames(
        &self,
        id: &MountId,
        frame: Option<String>,
        script: String,
    ) -> EngineFuture<'_, Option<Vec<vw_preview::FrameAnswer>>> {
        self.frames_with(id, frame, script, false)
    }
    fn evaluate_frames_as_input(
        &self,
        id: &MountId,
        frame: Option<String>,
        script: String,
    ) -> EngineFuture<'_, Option<Vec<vw_preview::FrameAnswer>>> {
        self.frames_with(id, frame, script, true)
    }
    fn native_click(
        &self,
        id: &MountId,
        point: vw_preview::NativeClickPoint,
        guard: vw_preview::NativeClickGuard,
    ) -> EngineFuture<'_, ()> {
        let id = id.clone();
        Box::pin(async move {
            if self.focus_required.load(Ordering::Relaxed) {
                return Err(EngineError::FocusRequired);
            }
            self.person.arm(&guard.person);
            if let Some(script) = &guard.validation {
                if let Ok(request) = serde_json::from_str::<Value>(script) {
                    let answers = self
                        .evaluate_frames(
                            &MountId {
                                session_id: SESSION.into(),
                                label: String::new(),
                            },
                            None,
                            request["script"].as_str().unwrap().into(),
                        )
                        .await?
                        .unwrap();
                    vw_preview::validate_frame_point(&request, &answers)
                        .map_err(EngineError::Platform)?;
                } else {
                    let valid = self.rpc(json!({"op":"evaluate", "script":script})).await?;
                    if valid != true {
                        return Err(EngineError::Platform("native validation refused".into()));
                    }
                }
            }
            if guard.allowed.as_ref().is_some_and(|allowed| !allowed()) {
                return Err(EngineError::Platform("native admission refused".into()));
            }
            let after_validation = self.after_point_validation.lock().unwrap().take();
            if let Some(script) = after_validation {
                self.rpc(json!({"op":"evaluate", "script":script})).await?;
            }
            if self
                .user_click_before_dispatch
                .swap(false, Ordering::Relaxed)
            {
                self.rpc(json!({"op":"click", "x":point.x, "y":point.y}))
                    .await?;
            }
            if let Some(validation) = &guard.validation {
                let request: Value = serde_json::from_str(validation).unwrap();
                let answers = self
                    .evaluate_frames(
                        &MountId {
                            session_id: SESSION.into(),
                            label: String::new(),
                        },
                        None,
                        request["dispatch"].as_str().unwrap().into(),
                    )
                    .await?
                    .unwrap();
                vw_preview::validate_frame_point(&request, &answers)
                    .map_err(EngineError::Platform)?;
            }
            if self.user_click_after_mark.swap(false, Ordering::Relaxed) {
                self.rpc(json!({"op":"click", "x":point.x, "y":point.y}))
                    .await?;
                self.rpc(json!({"op":"frameCoverReady"})).await?;
            }
            if self.person_click_after_mark.swap(false, Ordering::Relaxed) {
                self.person_click(point.x, point.y).await;
                if self.registry_lag.load(Ordering::Relaxed) {
                    self.rpc(json!({"op":"frameCoverReady"})).await?;
                }
            }
            self.person
                .admit(
                    &guard.person,
                    self.person_holding.load(Ordering::Relaxed),
                    PERSON_RECENT,
                )
                .map_err(EngineError::Platform)?;
            guard.sent.store(true, Ordering::Release);
            self.native_clicks.fetch_add(1, Ordering::Relaxed);
            if !self.agent_input_lost.load(Ordering::Relaxed) {
                self.agent_click(&id.label, point, &guard).await?;
            }
            if self
                .person_click_after_dispatch
                .swap(false, Ordering::Relaxed)
            {
                self.person_click(point.x, point.y).await;
            }
            if self
                .user_click_after_dispatch
                .swap(false, Ordering::Relaxed)
            {
                self.rpc(json!({"op":"click", "x":point.x, "y":point.y}))
                    .await?;
            }
            Ok(())
        })
    }
    fn attach(&self, hooks: Arc<dyn PreviewHooks>) {
        assert!(self.hooks.set(hooks).is_ok());
    }
    fn mount(&self, spec: MountSpec) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            self.load(&spec.id, spec.url, Some(spec.console_script))
                .await
        })
    }
    fn navigate(&self, id: &MountId, url: Url) -> EngineFuture<'_, ()> {
        let id = id.clone();
        Box::pin(async move { self.load(&id, url, None).await })
    }
    fn history(&self, _: &MountId, _: HistoryStep) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn place(&self, _: &MountId, _: Option<Placement>) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn close(&self, _: &MountId) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn drain_console(&self, _: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn wipe(&self, _: &Storage) -> EngineFuture<'_, Vec<PathBuf>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn screenshot(&self, id: &MountId) -> EngineFuture<'_, Vec<u8>> {
        let id = id.clone();
        Box::pin(async move {
            let value = self.rpc(json!({"op":"screenshot"})).await?;
            if self.navigate_during_shot.swap(false, Ordering::Relaxed) {
                self.hooks
                    .get()
                    .unwrap()
                    .page_event(&id.label, PageEvent::Started(URL.parse().unwrap()));
            }
            STANDARD
                .decode(value.as_str().unwrap())
                .map_err(|e| EngineError::Platform(e.to_string()))
        })
    }
    fn evaluate(&self, _: &MountId, script: String) -> EngineFuture<'_, String> {
        Box::pin(async move {
            let start = script.find("const REQ = ");
            let request = start.and_then(|at| {
                let req = &script[at + "const REQ = ".len()..];
                serde_json::from_str::<Value>(req.split(";\n").next()?).ok()
            });
            let pin = request.as_ref().is_some_and(|req| req["op"] == "pin");
            let value = self.rpc(json!({"op":"evaluate", "script":script})).await?;
            if request.as_ref().is_some_and(|r| r["action"] == "click")
                && value
                    .as_str()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .is_some_and(|v| v["status"] == "done")
                && let Some(action) = self.on_click.lock().unwrap().take()
            {
                action();
            }
            if request
                .as_ref()
                .is_some_and(|req| req["postconditionOnly"] == true)
            {
                let after_verify = self.after_verify.lock().unwrap().take();
                if let Some(script) = after_verify {
                    self.rpc(json!({"op":"evaluate", "script":script})).await?;
                }
            }
            if pin {
                self.capture_pin().await?;
            }
            Ok(value.to_string())
        })
    }
}

pub struct Rig {
    pub browser: Browser,
    pub service: PreviewService,
    pub engine: Arc<PageEngine>,
    pub dir: PathBuf,
}

impl Rig {
    pub async fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::var_os("VW_TEST_ROOT")
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join(format!(
                "browser-fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = PageEngine::new();
        let mut config = PreviewConfig::new(dir.join("preview"));
        config.engine_timeout = Duration::from_secs(120);
        config.name_resolver = Some(Arc::new(|_| Ok(vec!["93.184.215.14".parse().unwrap()])));
        let service = PreviewService::new(engine.clone(), config);
        service
            .open(OpenRequest {
                session_id: SESSION.into(),
                url: URL.parse().unwrap(),
                allowed_ports: vec![],
                downloads_dir: None,
                wait: true,
                lan: false,
            })
            .await
            .unwrap();
        service
            .layout(Some((
                SESSION.into(),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 300.0,
                    height: 200.0,
                },
            )))
            .await
            .unwrap();
        let browser = Browser::new(service.clone(), Arc::new(Sessions::default()));
        engine
            .eval("scrollTo({left:40, top:80, behavior:'instant'})")
            .await;
        Self {
            browser,
            service,
            engine,
            dir,
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub struct NoFetch;
impl Fetch for NoFetch {
    fn fetch<'a>(&'a self, _: &'a str, _: &'a str, _: u64) -> FetchFuture<'a> {
        Box::pin(async { Err("unexpected media fetch".into()) })
    }
}

impl PageEngine {
    /// `evaluate_frames`, or with `input` an agent act's own input (CDP: a user gesture with picker interception;
    /// a picker Chromium reported is passed to the hooks as refused, as the `WebView2` adapter does).
    fn frames_with(
        &self,
        id: &MountId,
        frame: Option<String>,
        script: String,
        input: bool,
    ) -> EngineFuture<'_, Option<Vec<vw_preview::FrameAnswer>>> {
        let label = id.label.clone();
        Box::pin(async move {
            let pickers = PICKERS.load(Ordering::Relaxed);
            let mut answers = self.frame_answers(frame.clone(), &script, input).await?;
            if input && PICKERS.load(Ordering::Relaxed) > pickers {
                self.hooks()
                    .act_refused(&label, vw_preview::ActRefusal::FilePicker);
            }
            if script.contains("JSON.stringify(state.result)") {
                let after_poll = self.after_receipt_poll.lock().unwrap().take();
                if let Some(script) = after_poll {
                    self.rpc(json!({"op":"evaluate", "script":script})).await?;
                }
                let user_click = self.user_click_after_poll.lock().unwrap().take();
                if let Some((x, y)) = user_click {
                    self.rpc(json!({"op":"click", "x":x, "y":y})).await?;
                }
            }
            let request = script
                .split_once("const REQ = ")
                .and_then(|(_, req)| serde_json::from_str::<Value>(req.split(";\n").next()?).ok());
            if request
                .as_ref()
                .is_some_and(|req| req["postconditionOnly"] == true)
            {
                let after_verify = self.after_verify.lock().unwrap().take();
                if let Some(script) = after_verify {
                    self.rpc(json!({"op":"evaluate", "script":script})).await?;
                }
            }
            if request.as_ref().is_some_and(|req| req["action"] == "click")
                && answers.iter().any(|a| {
                    serde_json::from_str::<String>(&a.value)
                        .ok()
                        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                        .is_some_and(|v| v["status"] == "done")
                })
                && let Some(action) = self.on_click.lock().unwrap().take()
            {
                action();
            }
            if request.as_ref().is_some_and(|req| req["op"] == "pin") {
                self.capture_pin().await?;
            }
            self.observe_faults(frame.is_none(), &script, &mut answers)?;
            if script.contains("JSON.stringify(state.result)")
                && self.unreadable_receipt.load(Ordering::Relaxed)
            {
                for answer in &mut answers {
                    answer.url = "file:///private".into(); // A web surface cannot read file frames.
                }
            }
            self.lag_frames(&mut answers);
            if self.reverse_frames.load(Ordering::Relaxed) {
                answers.reverse();
            }
            Ok(Some(answers))
        })
    }
}
