//! Real-browser contract for inline controls and the read-only prototype channel. Depends on a local Chromium driver.
//! Run explicitly (ignored by default) with `HB_INLINE_DRIVER` naming the Chromium driver; repositories must remain byte-identical after answering.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Fixture failures should stop the browser test immediately.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use url::Url;
use vw_pages::{AnswerValue, PagePath, PagesConfig, PagesService};
use vw_preview::{
    ConsoleEntry, EngineError, EngineFuture, HistoryStep, MountId, MountSpec, PageEvent, Placement,
    PreviewEngine, PreviewHooks, Rect, Storage,
};

struct Driver {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Driver {
    fn start() -> Self {
        let mut child = Command::new("node")
            .arg(
                std::env::var("HB_INLINE_DRIVER")
                    .expect("HB_INLINE_DRIVER names the Chromium fixture"),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
        }
    }

    fn call(&mut self, request: &Value) -> Value {
        writeln!(self.input, "{request}").unwrap();
        self.input.flush().unwrap();
        let mut response = String::new();
        self.output.read_line(&mut response).unwrap();
        let value: Value = serde_json::from_str(&response).expect("driver returned JSON");
        assert!(value.get("error").is_none(), "{value}");
        value["result"].clone()
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone)]
struct Browser {
    driver: Arc<Mutex<Driver>>,
    hooks: Arc<Mutex<Option<Arc<dyn PreviewHooks>>>>,
    quiet_loads: bool,
}

impl Browser {
    async fn call(&self, request: Value) -> Value {
        let driver = self.driver.clone();
        tokio::task::spawn_blocking(move || driver.lock().unwrap().call(&request))
            .await
            .unwrap()
    }
}

impl PreviewEngine for Browser {
    fn attach(&self, hooks: Arc<dyn PreviewHooks>) {
        *self.hooks.lock().unwrap() = Some(hooks);
    }
    fn mount(&self, spec: MountSpec) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            self.call(json!({"op":"mount", "url":spec.url.as_str(), "script":spec.console_script}))
                .await;
            if !self.quiet_loads {
                self.hooks
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .page_event(&spec.id.label, PageEvent::Finished(spec.url));
            }
            Ok(())
        })
    }
    fn navigate(&self, id: &MountId, url: Url) -> EngineFuture<'_, ()> {
        let (id, url) = (id.clone(), url.clone());
        Box::pin(async move {
            self.call(json!({"op":"navigate", "url":url.as_str()}))
                .await;
            if !self.quiet_loads {
                self.hooks
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .page_event(&id.label, PageEvent::Finished(url));
            }
            Ok(())
        })
    }
    fn evaluate(&self, _id: &MountId, script: String) -> EngineFuture<'_, String> {
        Box::pin(async move {
            Ok(self
                .call(json!({"op":"evaluate", "script":script}))
                .await
                .to_string())
        })
    }
    fn place(&self, _id: &MountId, placement: Option<Placement>) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            if placement.is_none() {
                self.call(json!({"op":"blur"})).await;
            }
            Ok(())
        })
    }
    fn close(&self, _id: &MountId) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn history(&self, _id: &MountId, _step: HistoryStep) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn screenshot(&self, _id: &MountId) -> EngineFuture<'_, Vec<u8>> {
        Box::pin(async {
            Err(EngineError::Unsupported(
                "fixture uses the browser capture".into(),
            ))
        })
    }
    fn drain_console(&self, _id: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn wipe(&self, _storage: &Storage) -> EngineFuture<'_, Vec<PathBuf>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

async fn wait_saved(
    service: &PagesService,
    project: &std::path::Path,
    path: &PagePath,
    count: u32,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if service.answers_of(project, path).unwrap().answered == count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("trusted browser answers persisted");
}

fn assert_repository_unchanged(folder: &std::path::Path, fixture: &Value) {
    for (name, source) in fixture.as_object().unwrap() {
        assert_eq!(
            std::fs::read_to_string(folder.join(name)).unwrap(),
            source.as_str().unwrap(),
            "answering never writes the repository"
        );
    }
}

async fn hidden_note_is_saved(
    service: &PagesService,
    browser: &Browser,
    project: &std::path::Path,
    board: &PagePath,
) {
    browser.call(json!({"op":"draftHidden"})).await;
    service.layout(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if matches!(service.answers_of(project, board).unwrap().inputs[3].answer.as_ref().unwrap().value, AnswerValue::Text { ref text } if text == "Saved after hide") { break; }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.expect("blur after native hide is persisted");
    service
        .layout(Some((
            "test".into(),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 1000.0,
            },
        )))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires the local Chromium driver in HB_INLINE_DRIVER"]
async fn trusted_inline_controls_and_a_resorting_sandbox() {
    let browser = Browser {
        driver: Arc::new(Mutex::new(Driver::start())),
        hooks: Arc::default(),
        quiet_loads: false,
    };
    let fixture = browser.call(json!({"op":"fixture"})).await;
    let root = std::env::temp_dir().join(format!("vw-inline-browser-{}", std::process::id()));
    let project = root.join("project");
    let folder = project.join("docs/handbook");
    std::fs::create_dir_all(&folder).unwrap();
    for (name, source) in fixture.as_object().unwrap() {
        std::fs::write(folder.join(name), source.as_str().unwrap()).unwrap();
    }
    let mut config = PagesConfig::new(root.join("data"), root.join("view"));
    config.load_wait = Duration::from_millis(10);
    let service = PagesService::new(Arc::new(browser.clone()), config);
    let board = PagePath::parse("board.html").unwrap();
    service
        .open("test", &project, Some(board.clone()))
        .await
        .unwrap();
    service
        .layout(Some((
            "test".into(),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 1000.0,
            },
        )))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let report = browser.call(json!({"op":"board"})).await;
    println!("board evidence: {report}");
    wait_saved(&service, &project, &board, 4).await;
    let saved = service.answers_of(&project, &board).unwrap();
    assert!(
        matches!(saved.inputs[3].answer.as_ref().unwrap().value, AnswerValue::Text { ref text } if text == "A useful note")
    );
    hidden_note_is_saved(&service, &browser, &project, &board).await;
    service
        .set_answer(
            &project,
            &board,
            "release",
            &AnswerValue::Choice {
                option: "later".into(),
            },
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(browser.call(json!({"op":"evaluate", "script":"document.querySelector('#release [data-option=later]').getAttribute('aria-pressed')"})).await, "true");
    let roadmap = PagePath::parse("roadmap.html").unwrap();
    browser.call(json!({"op":"depart"})).await;
    service
        .open("test", &project, Some(roadmap.clone()))
        .await
        .unwrap();
    assert!(
        matches!(service.answers_of(&project, &board).unwrap().inputs[3].answer.as_ref().unwrap().value, AnswerValue::Text { ref text } if text == "Saved before leaving")
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let report = browser.call(json!({"op":"prototype"})).await;
    println!("prototype evidence: {report}");
    wait_saved(&service, &project, &roadmap, 2).await;
    assert_eq!(
        service.answers_of(&project, &roadmap).unwrap().inputs.len(),
        250
    );
    assert_eq!(
        service
            .answers_of(&project, &roadmap)
            .unwrap()
            .detached
            .len(),
        0
    );
    let (pick_service, pick_project) = (service.clone(), project.clone());
    let pick = tokio::spawn(async move { pick_service.pick_section("test", &pick_project).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    browser.call(json!({"op":"clickSandbox"})).await;
    let picked = pick.await.unwrap().unwrap();
    assert_eq!(picked.section, "card-0");
    let comment = service
        .comments()
        .add(
            &project,
            &roadmap,
            (&picked.section, &picked.label),
            "Discuss this card",
            None,
        )
        .unwrap();
    assert_eq!(comment.section, "card-0");
    assert_repository_unchanged(&folder, &fixture);
    service.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[ignore = "requires the local Chromium driver in HB_INLINE_DRIVER"]
async fn step_fragments_scroll_and_rendered_documents_finish_without_native_load_events() {
    let browser = Browser {
        driver: Arc::new(Mutex::new(Driver::start())),
        hooks: Arc::default(),
        quiet_loads: true,
    };
    let root = std::env::temp_dir().join(format!("vw-step-browser-{}", std::process::id()));
    let project = root.join("project");
    let folder = project.join("docs/handbook");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("plan.html"), r#"<!doctype html><title>Step navigation</title>
        <meta name="handbook.kind" content="plan"><section id="goal">Goal</section>
        <div style="height:1800px"></div><ol class="hb-steps"><li data-step="s4" data-status="todo">Fourth step</li></ol>
        <div style="height:1200px"></div>"#).unwrap();
    let mut config = PagesConfig::new(root.join("data"), root.join("view"));
    config.load_wait = Duration::from_millis(400);
    let service = PagesService::new(Arc::new(browser.clone()), config);
    service
        .layout(Some((
            "viewer".into(),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 360.0,
                height: 600.0,
            },
        )))
        .await
        .unwrap();
    let path = PagePath::parse("plan.html").unwrap();
    service
        .open_at("viewer", &project, Some(path.clone()), Some("goal".into()))
        .await
        .unwrap();
    service
        .open_at("viewer", &project, Some(path.clone()), Some("s4".into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let position = browser.call(json!({"op":"evaluate", "script":"document.querySelector('[data-step=s4]').getBoundingClientRect().top"})).await;
    let view = service.view("viewer").unwrap();
    println!("step position={position}; viewer={view:?}");
    assert!(
        position.as_f64().unwrap().abs() < 3.0,
        "step must scroll into view: {position}"
    );
    assert!(!view.loading, "rendered document is loaded");
    assert_eq!(view.title.as_deref(), Some("Step navigation"));
    service
        .open_at("viewer", &project, Some(path), Some("s4".into()))
        .await
        .unwrap();
    assert_eq!(
        service.view("viewer").unwrap().title.as_deref(),
        Some("Step navigation")
    );
    service.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}
