//! App preview's own tools after the agent worked in a web tab: the agent's browser call brings its tab forward, and
//! that is not the person choosing a web tab. `preview_screenshot` selects App preview again (`preview_open` runs
//! the same reselect); a web tab the person selected stays, and the screenshot keeps asking them. A stand-in window
//! reports the selected page the way the window does (after every tab change), over an offline core and a
//! windowless engine.
//!
//! PREDICT (before the fix): the screenshot after the agent's tab came forward is refused with "The user has
//! selected a web tab instead of App preview", because the core kept one selection with no record of who made it
//! and only the window's own App preview request (not the screenshot) ever selected App preview again.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};
use vw_agent_tools::wire;
use vw_preview::{
    ConsoleEntry, EngineFuture, HistoryStep, MountId, MountSpec, OpenRequest, Placement,
    PreviewEngine, PreviewHooks, PreviewService, Rect, Storage, VisibilityReport,
};

const CHAT: &str = "chat";
const NOT_SELECTED: &str = "The user has selected a web tab instead of App preview.";

struct Engine;

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
        Box::pin(async {
            Ok(Some(vec![vw_preview::FrameAnswer {
                id: "main".to_owned(),
                generation: None,
                main: true,
                url: "http://127.0.0.1/".to_owned(),
                origin: None,
                value: json!({"shown": false, "frames": 0}).to_string(),
            }]))
        })
    }
}

/// What the window reports: Browser open on this chat, with the core's selected page.
fn report(service: &PreviewService) {
    let selected = service
        .tabs(CHAT)
        .into_iter()
        .find(|tab| tab.active)
        .map(|tab| tab.page_key);
    service.report_visibility(VisibilityReport {
        window_visible: true,
        selected_session_id: Some(CHAT.to_owned()),
        browser_panel_open: true,
        selected_page_key: selected.or_else(|| Some(CHAT.to_owned())),
    });
}

fn request(page: &str, port: u16) -> OpenRequest {
    OpenRequest {
        session_id: page.to_owned(),
        url: format!("http://127.0.0.1:{port}/").parse().unwrap(),
        allowed_ports: vec![port],
        downloads_dir: None,
        wait: false,
        lan: false,
    }
}

/// An offline core with App preview open and shown, one web tab, and a stand-in window. Returns the web tab's key
/// and the listener App preview's address answers on (kept open, so the preview never reads as a stopped server).
async fn core(name: &str) -> (crate::Core, String, std::net::TcpListener) {
    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.preview = Some(crate::PreviewSetup {
            engine: Arc::new(Engine),
            config: vw_preview::PreviewConfig::new(dir.join("preview")),
            guard: None,
        });
    })
    .await;
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":CHAT,"status":{"type":"idle"}},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    let service = core.shared.preview.as_ref().unwrap().service.clone();
    service.open(request(CHAT, port)).await.unwrap();
    let area = Rect {
        x: 600.0,
        y: 100.0,
        width: 520.0,
        height: 400.0,
    };
    service
        .layout_panel(Some(CHAT.to_owned()), Some(area))
        .await
        .unwrap();
    let web = service.create_web_tab(CHAT).unwrap();
    service.open(request(&web, port)).await.unwrap();
    report(&service);
    let window = service.clone();
    let mut changes = service.changes();
    tokio::spawn(async move {
        // A lagging receiver still reports the current selection, as the window re-reads it.
        while !matches!(
            changes.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        ) {
            report(&window);
        }
    });
    (core, web, server)
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

fn selected(core: &crate::Core) -> Option<String> {
    let service = &core.shared.preview.as_ref().unwrap().service;
    service
        .tabs(CHAT)
        .into_iter()
        .find(|tab| tab.active)
        .map(|tab| tab.page_key)
}

#[tokio::test]
async fn the_agents_own_web_tab_gives_way_to_app_previews_screenshot() {
    let (core, web, _server) = core("preview-select-agent").await;
    let service = core.shared.preview.as_ref().unwrap().service.clone();
    for round in 0..3 {
        // The agent's browser_act / browser_read brings its tab forward.
        crate::browser_tabs::front_page(&service, CHAT, &web)
            .await
            .unwrap();
        assert_eq!(selected(&core).as_deref(), Some(web.as_str()));
        match screenshot(&core).await {
            Ok(answer) => assert!(answer[wire::IMAGE_KEY]["data"].is_string(), "{answer}"),
            // Later rounds only: the offline core's process scan can withdraw the fixture's port from the chat (a
            // stopped-server answer), which is not this test's subject. The selection must never be the refusal.
            Err(refusal) => assert!(
                round > 0 && !refusal.contains(NOT_SELECTED),
                "round {round}: {refusal}"
            ),
        }
        assert_eq!(selected(&core).as_deref(), Some(CHAT), "round {round}");
    }
    core.shutdown().await;
}

#[tokio::test]
async fn a_web_tab_the_person_selected_stays_and_the_screenshot_asks_them() {
    let (core, web, _server) = core("preview-select-person").await;
    let service = core.shared.preview.as_ref().unwrap().service.clone();
    // The agent's tab came forward first; then the person clicked the web tab (the window's SwitchTab).
    crate::browser_tabs::front_page(&service, CHAT, &web)
        .await
        .unwrap();
    service.front_tab_if_visible(CHAT, &web).await.unwrap();
    let refusal = screenshot(&core).await.unwrap_err();
    assert!(refusal.contains(NOT_SELECTED), "{refusal}");
    assert_eq!(selected(&core).as_deref(), Some(web.as_str()));
    core.shutdown().await;
}
