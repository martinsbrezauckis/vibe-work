//! Main document replacement releases native placements while retaining live page state.
//! Uses recording engines without a window; called by the core crate's test runner.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vw_pages::{PagePath, PagesConfig, PagesService};
use vw_preview::{
    ConsoleEntry, EngineFuture, HistoryStep, MountId, MountSpec, OpenRequest, Placement,
    PreviewConfig, PreviewEngine, PreviewHooks, PreviewService, Rect, Storage,
};

const AREA: Rect = Rect {
    x: 600.0,
    y: 100.0,
    width: 520.0,
    height: 400.0,
};

#[derive(Default)]
struct Engine {
    placements: Mutex<Vec<Option<Placement>>>,
    overlays: Mutex<Vec<bool>>,
    hooks: Mutex<Option<Arc<dyn PreviewHooks>>>,
    mounts: Mutex<usize>,
}

impl PreviewEngine for Engine {
    fn attach(&self, hooks: Arc<dyn PreviewHooks>) {
        *self.hooks.lock().unwrap() = Some(hooks);
    }
    fn mount(&self, spec: MountSpec) -> EngineFuture<'_, ()> {
        *self.mounts.lock().unwrap() += 1;
        self.placements.lock().unwrap().push(spec.placement);
        Box::pin(async { Ok(()) })
    }
    fn navigate(&self, _: &MountId, _: url::Url) -> EngineFuture<'_, ()> {
        panic!("pages must stay alive")
    }
    fn history(&self, _: &MountId, _: HistoryStep) -> EngineFuture<'_, ()> {
        panic!("documents must not reload")
    }
    fn place(&self, _: &MountId, placement: Option<Placement>) -> EngineFuture<'_, ()> {
        self.placements.lock().unwrap().push(placement);
        Box::pin(async { Ok(()) })
    }
    fn browser_overlay(&self, _: &MountId, hidden: bool) -> EngineFuture<'_, ()> {
        self.overlays.lock().unwrap().push(hidden);
        Box::pin(async { Ok(()) })
    }
    fn close(&self, _: &MountId) -> EngineFuture<'_, ()> {
        panic!("pages must stay mounted")
    }
    fn screenshot(&self, _: &MountId) -> EngineFuture<'_, Vec<u8>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn drain_console(&self, _: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn wipe(&self, _: &Storage) -> EngineFuture<'_, Vec<PathBuf>> {
        panic!("storage must be kept")
    }
}

async fn preview_reset(page: &str, web: bool, overlay: bool) {
    let engine = Arc::new(Engine::default());
    let service = PreviewService::new(engine.clone(), PreviewConfig::new(PathBuf::new()));
    let page = if web {
        service.create_web_tab("chat").unwrap()
    } else {
        page.to_owned()
    };
    if web {
        service.set_agent_page("chat", &page).unwrap();
        service.switch_tab("chat", &page).unwrap();
    }
    service
        .open(OpenRequest {
            session_id: page.clone(),
            url: "http://127.0.0.1:28107/".parse().unwrap(),
            allowed_ports: vec![28107],
            downloads_dir: None,
            wait: false,
            lan: false,
        })
        .await
        .unwrap();
    service
        .layout_panel(Some(page.clone()), Some(AREA))
        .await
        .unwrap();
    assert_eq!(
        service.view(&page).unwrap().state,
        vw_preview::PreviewState::Live
    );
    if overlay {
        service
            .prepare_browser_overlay("chat", &page)
            .await
            .unwrap();
        service.show_browser_overlay("chat", &page).await.unwrap();
    }
    let before = service.view(&page).unwrap();
    let mut tabs = service.tabs("chat");
    for tab in &mut tabs {
        tab.preview = None;
    }
    let agent = service.agent_page("chat");

    super::reset(Some(&service), None, None).await;

    assert_eq!(
        service.view(&page).unwrap().state,
        vw_preview::PreviewState::Hidden,
        "the replacement UI owns no placement"
    );
    assert_eq!(engine.placements.lock().unwrap().last(), Some(&None));
    assert_eq!(service.view(&page).unwrap().url, before.url);
    let mut after_tabs = service.tabs("chat");
    for tab in &mut after_tabs {
        tab.preview = None;
    }
    assert_eq!(after_tabs, tabs);
    assert_eq!(service.agent_page("chat"), agent);
    assert!(service.visible_browser_area("chat").is_none());
    if overlay {
        assert_eq!(engine.overlays.lock().unwrap().as_slice(), &[true, false]);
        let released = engine.overlays.lock().unwrap().len();
        service.close_browser_overlay("chat", &page).await.unwrap();
        assert_eq!(
            engine.overlays.lock().unwrap().len(),
            released,
            "old overlays are forgotten"
        );
    }
    service
        .layout_panel(Some(page.clone()), Some(AREA))
        .await
        .unwrap();
    assert_eq!(
        service.view(&page).unwrap().state,
        vw_preview::PreviewState::Live
    );
    assert_eq!(*engine.mounts.lock().unwrap(), 1);
}

#[tokio::test]
async fn replacing_the_main_document_hides_the_app_preview() {
    preview_reset("chat", false, false).await;
}

#[tokio::test]
async fn replacing_the_main_document_hides_web_tabs_and_preserves_agent_ownership() {
    preview_reset("", true, false).await;
}

#[tokio::test]
async fn replacing_the_main_document_hides_the_accounts_page() {
    preview_reset(crate::browser_accounts::ACCOUNT_PAGE, false, false).await;
}

#[tokio::test]
async fn replacing_the_main_document_releases_browser_overlays_without_restoring_the_page() {
    preview_reset("", true, true).await;
}

async fn viewer_reset(files: bool) {
    let root = std::env::temp_dir().join(format!("viewer-reload-{}-{files}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("page.html"), "<!doctype html><title>Page</title>").unwrap();
    let engine = Arc::new(Engine::default());
    let mut config = if files {
        PagesConfig::for_files(root.join("data"), root.join("view"))
    } else {
        PagesConfig::new(root.join("data"), root.join("view"))
    };
    config.load_wait = Duration::ZERO;
    let service = PagesService::new(engine.clone(), config);
    service
        .layout(Some(("chat".to_owned(), AREA)))
        .await
        .unwrap();
    service
        .open_file("chat", &root, PagePath::parse("page.html").unwrap())
        .await
        .unwrap();
    assert!(service.view("chat").unwrap().shown);
    let before = service.view("chat").unwrap();

    if files {
        super::reset(None, None, Some(&service)).await;
    } else {
        super::reset(None, Some(&service), None).await;
    }

    assert!(
        !service.view("chat").unwrap().shown,
        "the replacement UI owns no placement"
    );
    assert_eq!(engine.placements.lock().unwrap().last(), Some(&None));
    assert_eq!(service.view("chat").unwrap().path, before.path);
    service
        .layout(Some(("chat".to_owned(), AREA)))
        .await
        .unwrap();
    assert!(service.view("chat").unwrap().shown);
    assert_eq!(*engine.mounts.lock().unwrap(), 1);
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn replacing_the_main_document_hides_the_project_viewer() {
    viewer_reset(false).await;
}

#[tokio::test]
async fn replacing_the_main_document_hides_the_file_viewer() {
    viewer_reset(true).await;
}
