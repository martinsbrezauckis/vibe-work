//! Real page fixture for browser regressions, driven through `PreviewService`.
//! Depends on Node, playwright-core and its installed Chromium; called by browser integration tests.
//! Each test owns its subprocess, storage and page; page scripts are the production scripts.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tokio::sync::{Notify, Semaphore};
use url::Url;
use vw_browser::capture::{Fetch, FetchFuture};
use vw_browser::{Browser, Sessions};
use vw_preview::{
    ConsoleEntry, EngineError, EngineFuture, HistoryStep, MountId, MountSpec, OpenRequest,
    PageEvent, Placement, PreviewConfig, PreviewEngine, PreviewHooks, PreviewService, Rect,
    Storage,
};

pub const SESSION: &str = "fixture";
pub const URL: &str = "https://93.184.215.14/";

struct Pipe {
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

pub struct PageEngine {
    pipe: Arc<Mutex<Pipe>>,
    hooks: OnceLock<Arc<dyn PreviewHooks>>,
    pub hold_pin: AtomicBool,
    pub fail_pin: AtomicBool,
    pub pinned: Notify,
    pub release: Semaphore,
    pub on_pin: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    pub native_clicks: AtomicU64,
    pub after_verify: Mutex<Option<String>>,
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
        let mut child = command.spawn().expect("Node fixture");
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Arc::new(Self {
            pipe: Arc::new(Mutex::new(Pipe {
                child,
                input,
                output,
            })),
            hooks: OnceLock::new(),
            hold_pin: AtomicBool::new(false),
            fail_pin: AtomicBool::new(false),
            pinned: Notify::new(),
            release: Semaphore::new(0),
            on_pin: Mutex::new(None),
            native_clicks: AtomicU64::new(0),
            after_verify: Mutex::new(None),
        })
    }

    async fn rpc(&self, request: Value) -> Result<Value, EngineError> {
        let pipe = self.pipe.clone();
        tokio::task::spawn_blocking(move || {
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
        })
        .await
        .map_err(|e| EngineError::Platform(e.to_string()))?
    }

    pub async fn eval(&self, script: &str) -> Value {
        self.rpc(json!({"op":"evaluate", "script":script}))
            .await
            .unwrap()
    }

    pub async fn snapshot(&self) -> Value {
        self.eval("(() => { const nodes = [document.querySelector('#styled'), document.querySelector('#bare'), document.querySelector('#shadow').shadowRoot.querySelector('#inner')]; return { x:scrollX, y:scrollY, styles:nodes.map(e => e.getAttribute('style')), pins:nodes.map(e => e.hasAttribute('data-vw-capture-pin')) }; })()").await
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
    fn native_click(
        &self,
        _: &MountId,
        point: vw_preview::NativeClickPoint,
        guard: vw_preview::NativeClickGuard,
    ) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            if let Some(script) = &guard.validation {
                let valid = self.rpc(json!({"op":"evaluate", "script":script})).await?;
                if valid != true {
                    return Err(EngineError::Platform("native validation refused".into()));
                }
            }
            if guard.allowed.as_ref().is_some_and(|allowed| !allowed()) {
                return Err(EngineError::Platform("native admission refused".into()));
            }
            guard.sent.store(true, Ordering::Release);
            self.native_clicks.fetch_add(1, Ordering::Relaxed);
            self.rpc(json!({"op":"click", "x":point.x, "y":point.y}))
                .await?;
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
    fn screenshot(&self, _: &MountId) -> EngineFuture<'_, Vec<u8>> {
        Box::pin(async {
            let value = self.rpc(json!({"op":"screenshot"})).await?;
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
                if let Some(action) = self.on_pin.lock().unwrap().take() {
                    action();
                }
                self.pinned.notify_one();
                if self.fail_pin.load(Ordering::Relaxed) {
                    return Err(EngineError::Platform("forced slice failure".into()));
                }
                if self.hold_pin.load(Ordering::Relaxed) {
                    self.release.acquire().await.unwrap().forget();
                }
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
        let dir = std::env::temp_dir().join(format!(
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
