//! App preview "Annotate this page" obeys the protected-value visual-capture admission (`vw_browser::Browser::admit_capture`):
//! below Full access (Ask, Accept edits) a page showing a password, PIN or card value omits the crop picture and returns
//! the shared Capture-menu refusal in `crop_left_out` while preserving the element/box note and coordinates; Full access,
//! safe/masked pages, and an independent second Full-access chat still receive the crop. Uses an offline core and a
//! windowless engine.
//!
//! PREDICT (before the admission): `BrowserRequest::Annotate` below Full access returns `crop_png_base64: Some(...)`
//! and `crop_left_out: None` even while the page reports a visible protected field, calling `Engine::screenshot`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;
use url::Url;
use vw_preview::{
    ConsoleEntry, EngineFuture, FrameAnswer, MountId, MountSpec, OpenRequest, Placement,
    PreviewEngine, PreviewHooks, Rect, Storage,
};

use crate::{BrowserRequest, BrowserResult, Command, CommandResult, Origin};

struct Engine {
    shown_by_label: Mutex<HashMap<String, bool>>,
    flip_on_shot: Mutex<Option<String>>,
    shots: AtomicUsize,
    fail_admit_for: Mutex<Option<String>>,
    box_answer: AtomicBool,
}

impl Engine {
    fn new() -> Self {
        Self {
            shown_by_label: Mutex::new(HashMap::new()),
            flip_on_shot: Mutex::new(None),
            shots: AtomicUsize::new(0),
            fail_admit_for: Mutex::new(None),
            box_answer: AtomicBool::new(false),
        }
    }

    fn set_shown(&self, label: &str, shown: bool) {
        self.shown_by_label
            .lock()
            .unwrap()
            .insert(label.to_owned(), shown);
    }
}

impl PreviewEngine for Engine {
    fn attach(&self, _: Arc<dyn PreviewHooks>) {}
    fn mount(&self, _: MountSpec) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn navigate(&self, _: &MountId, _: Url) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn history(&self, _: &MountId, _: vw_preview::HistoryStep) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn place(&self, _: &MountId, _: Option<Placement>) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn close(&self, _: &MountId) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn screenshot(&self, id: &MountId) -> EngineFuture<'_, Vec<u8>> {
        self.shots.fetch_add(1, Ordering::SeqCst);
        if self.flip_on_shot.lock().unwrap().as_deref() == Some(id.session_id.as_str()) {
            self.set_shown(&id.session_id, true);
        }
        let image = vw_preview::Rgba {
            width: 320,
            height: 240,
            pixels: vec![120; 320 * 240 * 4],
        };
        let bytes = vw_preview::encode(&image).unwrap();
        Box::pin(async move { Ok(bytes) })
    }
    fn evaluate(&self, id: &MountId, script: String) -> EngineFuture<'_, String> {
        let answer = if script.contains("__vw_annotate_") {
            let mode = if self.box_answer.load(Ordering::SeqCst) {
                "box"
            } else {
                "element"
            };
            json!({
                "state": "picked",
                "mode": mode,
                "ref": "e00000001",
                "role": "button",
                "name": "Pay now",
                "text": "Pay now",
                "path": "main > button",
                "url": format!("http://127.0.0.1/?chat={}", id.session_id),
                "route": "/checkout",
                "rect": {"x": 20, "y": 30, "width": 100, "height": 40},
                "viewport": {"width": 320, "height": 240}
            })
            .to_string()
        } else {
            "null".to_owned()
        };
        Box::pin(async move { Ok(answer) })
    }
    fn evaluate_frames(
        &self,
        id: &MountId,
        _: Option<String>,
        script: String,
    ) -> EngineFuture<'_, Option<Vec<FrameAnswer>>> {
        if script.contains("vibework-closed-roots-ready") {
            if self.fail_admit_for.lock().unwrap().as_deref() == Some(id.session_id.as_str()) {
                return Box::pin(async {
                    Err(vw_preview::EngineError::Platform(
                        "frame probe failed".to_owned(),
                    ))
                });
            }
            let shown = *self
                .shown_by_label
                .lock()
                .unwrap()
                .get(&id.session_id)
                .unwrap_or(&false);
            return Box::pin(async move {
                Ok(Some(vec![FrameAnswer {
                    id: "main".to_owned(),
                    generation: None,
                    main: true,
                    url: "http://127.0.0.1/".to_owned(),
                    origin: None,
                    value: json!({ "shown": shown, "frames": 0 }).to_string(),
                }]))
            });
        }
        let label = id.clone();
        Box::pin(async move {
            let value = self.evaluate(&label, script).await?;
            Ok(Some(vec![FrameAnswer {
                id: "main".to_owned(),
                generation: None,
                main: true,
                url: "http://127.0.0.1/".to_owned(),
                origin: None,
                value,
            }]))
        })
    }
    fn drain_console(&self, _: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn wipe(&self, _: &Storage) -> EngineFuture<'_, Vec<PathBuf>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

async fn setup_chat(
    core: &crate::Core,
    session: &str,
    port: u16,
    agent_mode: &str,
    channel_id: u64,
) -> tokio::io::DuplexStream {
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":session,"status":{"type":"idle"}},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    let (client, server) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(client);
    let connection =
        vw_rpc::ConnectionBuilder::new(vw_rpc::ConnectionConfig::default()).start(read, write);
    core.shared
        .channels
        .bind_test_peer(session, connection, (0, channel_id));
    core.shared
        .access
        .observe_runtime(session, &json!({"activeAgent":{"name":agent_mode}}));
    let service = &core.shared.preview.as_ref().unwrap().service;
    service
        .open(OpenRequest {
            session_id: session.to_owned(),
            url: format!("http://127.0.0.1:{port}/").parse().unwrap(),
            allowed_ports: vec![port],
            downloads_dir: None,
            wait: false,
            lan: false,
        })
        .await
        .unwrap();
    let area = Rect {
        x: 600.0,
        y: 100.0,
        width: 320.0,
        height: 240.0,
    };
    service
        .layout_panel(Some(session.to_owned()), Some(area))
        .await
        .unwrap();
    server
}

async fn focus_chat(core: &crate::Core, session: &str) {
    let service = &core.shared.preview.as_ref().unwrap().service;
    let area = Rect {
        x: 600.0,
        y: 100.0,
        width: 320.0,
        height: 240.0,
    };
    service
        .layout_panel(Some(session.to_owned()), Some(area))
        .await
        .unwrap();
}

async fn run_annotate(
    core: &crate::Core,
    session: &str,
    mode: Option<&str>,
) -> vw_browser::annotate::Annotation {
    let result = core
        .execute(
            Origin::Ui,
            Command::Browser {
                request: BrowserRequest::Annotate {
                    session_id: session.to_owned(),
                    mode: mode.map(str::to_owned),
                    target: None,
                },
            },
        )
        .await
        .unwrap();
    let CommandResult::Browser {
        result: BrowserResult::Annotation { annotation },
    } = result
    else {
        panic!("expected annotation result, got {result:?}");
    };
    annotation
}

#[tokio::test]
async fn annotate_crop_follows_protected_value_admission_across_modes_and_chats() {
    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let engine = Arc::new(Engine::new());
    let eng = engine.clone();
    let core = crate::offline_core::start_with("annotate-protected", |config, dir| {
        let mut preview = vw_preview::PreviewConfig::new(dir.join("preview"));
        // Needs two slots even on the 7 GiB macOS CI host.
        preview.memory_probe = Some(Arc::new(|| (Some(8), Some(4000))));
        config.preview = Some(crate::PreviewSetup {
            engine: eng,
            config: preview,
            guard: None,
        });
    })
    .await;

    let chat_a = "chat-a";
    let chat_b = "chat-b";
    let _peer_a = setup_chat(&core, chat_a, port, "ask", 1).await;
    let _peer_b = setup_chat(&core, chat_b, port, "auto-approve", 2).await;

    // Visible protected value below Full access (Ask) -> no crop, shared refusal wording, position kept, zero shots.
    engine.set_shown(chat_a, true);
    focus_chat(&core, chat_a).await;
    let shots_before = engine.shots.load(Ordering::SeqCst);
    let ask_protected = run_annotate(&core, chat_a, Some("element")).await;
    assert!(ask_protected.crop_png_base64.is_none());
    assert_eq!(
        ask_protected.crop_left_out.as_deref(),
        Some(vw_browser::protected_shown().as_str())
    );
    assert_eq!(ask_protected.mode, "element");
    assert_eq!(ask_protected.role, "button");
    assert_eq!(ask_protected.name, "Pay now");
    assert_eq!(ask_protected.route, "/checkout");
    assert_eq!(ask_protected.rect.width, 100);
    assert_eq!(engine.shots.load(Ordering::SeqCst), shots_before);

    // Concurrent second chat in Full access (auto-approve) keeps the crop even while its page also shows a secret.
    engine.set_shown(chat_b, true);
    focus_chat(&core, chat_b).await;
    let shots_before = engine.shots.load(Ordering::SeqCst);
    let full_protected = run_annotate(&core, chat_b, Some("element")).await;
    assert!(full_protected.crop_png_base64.is_some());
    assert!(full_protected.crop_left_out.is_none());
    assert_eq!(engine.shots.load(Ordering::SeqCst), shots_before + 1);

    // Accept edits (accept-edits) is also below Full access -> no crop (box mode included).
    core.shared
        .access
        .observe_runtime(chat_a, &json!({"activeAgent":{"name":"accept-edits"}}));
    engine.box_answer.store(true, Ordering::SeqCst);
    focus_chat(&core, chat_a).await;
    let shots_before = engine.shots.load(Ordering::SeqCst);
    let accept_protected = run_annotate(&core, chat_a, Some("box")).await;
    assert!(accept_protected.crop_png_base64.is_none());
    assert_eq!(
        accept_protected.crop_left_out.as_deref(),
        Some(vw_browser::protected_shown().as_str())
    );
    assert_eq!(accept_protected.mode, "box");
    assert_eq!(accept_protected.rect.height, 40);
    assert_eq!(engine.shots.load(Ordering::SeqCst), shots_before);
    engine.box_answer.store(false, Ordering::SeqCst);

    // Unchecked frame admission (evaluation error) below Full access refuses the crop safely too.
    engine.set_shown(chat_a, false);
    *engine.fail_admit_for.lock().unwrap() = Some(chat_a.to_owned());
    let fail_probe = run_annotate(&core, chat_a, Some("element")).await;
    assert!(fail_probe.crop_png_base64.is_none());
    assert_eq!(
        fail_probe.crop_left_out.as_deref(),
        Some(vw_browser::protected_shown().as_str())
    );
    *engine.fail_admit_for.lock().unwrap() = None;

    // Masked/hidden/ordinary page below Full access keeps the crop.
    engine.set_shown(chat_a, false);
    let ask_plain = run_annotate(&core, chat_a, Some("element")).await;
    assert!(ask_plain.crop_png_base64.is_some());
    assert!(ask_plain.crop_left_out.is_none());

    // A protected value shown while screenshot renders (TOCTOU) drops the crop afterwards.
    *engine.flip_on_shot.lock().unwrap() = Some(chat_a.to_owned());
    let toctou = run_annotate(&core, chat_a, Some("element")).await;
    assert!(toctou.crop_png_base64.is_none());
    assert_eq!(
        toctou.crop_left_out.as_deref(),
        Some(vw_browser::protected_shown().as_str())
    );
    assert_eq!(toctou.rect.x, 20);
    core.shutdown().await;
}
