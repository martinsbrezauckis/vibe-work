//! `preview_screenshot` returns App preview pixels only through the built-in browser's protected-value admission:
//! a page that shows a password, PIN or card value is refused in every access mode with the same text as every other
//! visual capture. Uses an offline core and a windowless engine whose page
//! script answers stand in for the shared privateField rule (its own behaviour is covered by vw-browser's
//! protected capture tests on real pixels).
//!
//! PREDICT (before the admission): the screenshot below Full access returns the image although the page reports a
//! shown protected value, because the tool took the preview service's screenshot directly and never ran the
//! admission script (`evaluations` stays 0).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde_json::{Value, json};
use vw_agent_tools::wire;
use vw_preview::{
    ConsoleEntry, EngineFuture, HistoryStep, MountId, MountSpec, OpenRequest, Placement,
    PreviewEngine, PreviewHooks, Rect, Storage,
};

const CHAT: &str = "chat";

#[derive(Default)]
struct Engine {
    /// The page shows a protected value (what the privateField rule would answer).
    shown: AtomicBool,
    evaluations: AtomicUsize,
}

impl PreviewEngine for Engine {
    fn attach(&self, _: Arc<dyn PreviewHooks>) {}
    fn mount(&self, _: MountSpec) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn navigate(&self, _: &MountId, _: url::Url) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
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
    fn screenshot(&self, _: &MountId) -> EngineFuture<'_, Vec<u8>> {
        Box::pin(async {
            let image = vw_preview::Rgba {
                width: 8,
                height: 8,
                pixels: vec![255; 8 * 8 * 4],
            };
            Ok(vw_preview::encode(&image).unwrap())
        })
    }
    fn drain_console(&self, _: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn wipe(&self, _: &Storage) -> EngineFuture<'_, Vec<PathBuf>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn evaluate_frames(
        &self,
        _: &MountId,
        _: Option<String>,
        _: String,
    ) -> EngineFuture<'_, Option<Vec<vw_preview::FrameAnswer>>> {
        self.evaluations.fetch_add(1, Ordering::SeqCst);
        let shown = self.shown.load(Ordering::SeqCst);
        Box::pin(async move {
            Ok(Some(vec![vw_preview::FrameAnswer {
                id: "main".to_owned(),
                generation: None,
                main: true,
                url: "http://127.0.0.1:28107/".to_owned(),
                origin: None,
                value: json!({"shown": shown, "frames": 0}).to_string(),
            }]))
        })
    }
}

async fn core(name: &str, engine: Arc<Engine>) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.preview = Some(crate::PreviewSetup {
            engine,
            config: vw_preview::PreviewConfig::new(dir.join("preview")),
            guard: None,
        });
    })
    .await;
    let shared = &core.shared;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":CHAT,"status":{"type":"idle"}},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    let hub = shared.preview.as_ref().unwrap();
    hub.service
        .open(OpenRequest {
            session_id: CHAT.to_owned(),
            url: "http://127.0.0.1:28107/".parse().unwrap(),
            allowed_ports: vec![28107],
            downloads_dir: None,
            wait: false,
            lan: false,
        })
        .await
        .unwrap();
    let area = Rect {
        x: 600.0,
        y: 100.0,
        width: 520.0,
        height: 400.0,
    };
    hub.service
        .layout_panel(Some(CHAT.to_owned()), Some(area))
        .await
        .unwrap();
    core
}

async fn screenshot(core: &crate::Core) -> Result<Value, String> {
    super::call(
        &core.shared,
        CHAT,
        wire::tools::PREVIEW_SCREENSHOT,
        &json!({}),
    )
    .await
}

#[tokio::test]
async fn app_preview_screenshot_below_full_access_is_refused_while_a_protected_value_shows() {
    let engine = Arc::new(Engine::default());
    let core = core("preview-shot-protected", engine.clone()).await;

    engine.shown.store(true, Ordering::SeqCst);
    assert_eq!(
        screenshot(&core).await.unwrap_err(),
        vw_browser::protected_shown(),
        "a shown card or PIN value is never returned without the delegation"
    );
    assert!(engine.evaluations.load(Ordering::SeqCst) > 0);

    engine.shown.store(false, Ordering::SeqCst);
    engine.evaluations.store(0, Ordering::SeqCst);
    let answer = screenshot(&core).await.unwrap();
    assert!(answer[wire::IMAGE_KEY]["data"].is_string(), "{answer}");
    assert_eq!(
        engine.evaluations.load(Ordering::SeqCst),
        2,
        "the admission runs before and after the pixels"
    );

    core.shutdown().await;
}

#[tokio::test]
async fn app_preview_screenshot_in_full_access_withholds_a_shown_value() {
    let engine = Arc::new(Engine::default());
    let core = core("preview-shot-full", engine.clone()).await;
    core.shared
        .access
        .observe_runtime(CHAT, &json!({"activeAgent":{"name":"auto-approve"}}));

    engine.shown.store(true, Ordering::SeqCst);
    assert_eq!(
        screenshot(&core).await.unwrap_err(),
        vw_browser::protected_shown()
    );

    core.shutdown().await;
}
