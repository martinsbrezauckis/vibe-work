//! Main document replacement releases native placements while retaining live page state, and a placement the
//! replaced document started never shows its child after the reset.
//! Uses recording engines without a window; called by the core crate's test runner.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
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
    /// The next mount reports that it started, then waits for the release before its child appears.
    held_mount: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
}

impl PreviewEngine for Engine {
    fn attach(&self, hooks: Arc<dyn PreviewHooks>) {
        *self.hooks.lock().unwrap() = Some(hooks);
    }
    fn mount(&self, spec: MountSpec) -> EngineFuture<'_, ()> {
        *self.mounts.lock().unwrap() += 1;
        let held = self.held_mount.lock().unwrap().take();
        Box::pin(async move {
            if let Some((started, release)) = held {
                started.send(()).unwrap();
                release.await.unwrap();
            }
            self.placements.lock().unwrap().push(spec.placement);
            Ok(())
        })
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

    super::reset(1, Some(&service), None, None).await;

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
        super::reset(1, None, None, Some(&service)).await;
    } else {
        super::reset(1, None, Some(&service), None).await;
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

/// The Browser page `page` placed as the window does: the accounts page by its own layout, any other page by a
/// tab layout report.
async fn place_browser(
    service: &PreviewService,
    page: &str,
) -> Result<(), vw_preview::PreviewError> {
    if page == crate::browser_accounts::ACCOUNT_PAGE {
        service.layout(Some((page.to_owned(), AREA))).await
    } else {
        service
            .report_tab_layout(Some(page.to_owned()), Some(AREA))
            .await
    }
}

async fn stale_browser_layout(page: &str, web: bool) {
    let engine = Arc::new(Engine::default());
    let service = PreviewService::new(engine.clone(), PreviewConfig::new(PathBuf::new()));
    let page = if web {
        let page = service.create_web_tab("chat").unwrap();
        service.switch_tab("chat", &page).unwrap();
        page
    } else {
        page.to_owned()
    };
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
    super::reset(1, Some(&service), None, None).await;
    super::for_window_generation(1, place_browser(&service, &page))
        .await
        .unwrap();
    assert_eq!(
        service.view(&page).unwrap().state,
        vw_preview::PreviewState::Live
    );

    // A layout the first document sent waits (as in session admission) until the reload's reset has run.
    let (release, held) = oneshot::channel::<()>();
    let stale = super::for_window_generation(1, async {
        held.await.unwrap();
        place_browser(&service, &page).await
    });
    let reload = async {
        super::reset(2, Some(&service), None, None).await;
        release.send(()).unwrap();
    };
    let (stale, ()) = tokio::join!(stale, reload);
    stale.unwrap();

    assert_eq!(
        service.view(&page).unwrap().state,
        vw_preview::PreviewState::Hidden,
        "the replaced document's layout shows nothing over the new one"
    );
    assert_eq!(engine.placements.lock().unwrap().last(), Some(&None));
    assert!(service.visible_browser_area("chat").is_none());

    super::for_window_generation(2, place_browser(&service, &page))
        .await
        .unwrap();
    assert_eq!(
        service.view(&page).unwrap().state,
        vw_preview::PreviewState::Live,
        "the new document places its page"
    );
    assert_eq!(*engine.mounts.lock().unwrap(), 1);
}

#[tokio::test]
async fn a_layout_of_the_replaced_document_keeps_the_app_preview_hidden() {
    stale_browser_layout("chat", false).await;
}

#[tokio::test]
async fn a_layout_of_the_replaced_document_keeps_a_web_tab_hidden() {
    stale_browser_layout("", true).await;
}

#[tokio::test]
async fn a_layout_of_the_replaced_document_keeps_the_accounts_page_hidden() {
    stale_browser_layout(crate::browser_accounts::ACCOUNT_PAGE, false).await;
}

/// A viewer service over `engine` with one page of `chat` opened while the viewer is hidden.
async fn viewer_with_page(
    engine: &Arc<Engine>,
    root: &std::path::Path,
    files: bool,
) -> PagesService {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("page.html"), "<!doctype html><title>Page</title>").unwrap();
    let mut config = if files {
        PagesConfig::for_files(root.join("data"), root.join("view"))
    } else {
        PagesConfig::new(root.join("data"), root.join("view"))
    };
    config.load_wait = Duration::ZERO;
    let service = PagesService::new(engine.clone(), config);
    super::reset(1, None, Some(&service), None).await;
    service
        .open_file("chat", root, PagePath::parse("page.html").unwrap())
        .await
        .unwrap();
    assert!(!service.view("chat").unwrap().shown);
    service
}

async fn stale_viewer_layout(files: bool) {
    let root = std::env::temp_dir().join(format!("viewer-stale-{}-{files}", std::process::id()));
    let engine = Arc::new(Engine::default());
    let service = viewer_with_page(&engine, &root, files).await;
    super::for_window_generation(1, service.layout(Some(("chat".to_owned(), AREA))))
        .await
        .unwrap();
    assert!(service.view("chat").unwrap().shown);

    let (release, held) = oneshot::channel::<()>();
    let stale = super::for_window_generation(1, async {
        held.await.unwrap();
        service.layout(Some(("chat".to_owned(), AREA))).await
    });
    let reload = async {
        super::reset(2, None, Some(&service), None).await;
        release.send(()).unwrap();
    };
    let (stale, ()) = tokio::join!(stale, reload);
    stale.unwrap();

    assert!(
        !service.view("chat").unwrap().shown,
        "the replaced document's layout shows nothing over the new one"
    );
    assert_eq!(engine.placements.lock().unwrap().last(), Some(&None));
    super::for_window_generation(2, service.layout(Some(("chat".to_owned(), AREA))))
        .await
        .unwrap();
    assert!(
        service.view("chat").unwrap().shown,
        "the new document places its page"
    );
    assert_eq!(
        engine.placements.lock().unwrap().last(),
        Some(&Some(
            vw_preview::place(AREA, vw_preview::DeviceWidth::Fit).unwrap()
        ))
    );
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_layout_of_the_replaced_document_keeps_the_project_viewer_hidden() {
    stale_viewer_layout(false).await;
}

#[tokio::test]
async fn a_layout_of_the_replaced_document_keeps_the_file_viewer_hidden() {
    stale_viewer_layout(true).await;
}

async fn viewer_mount_across_reset(files: bool) {
    let root = std::env::temp_dir().join(format!("viewer-mounting-{}-{files}", std::process::id()));
    let engine = Arc::new(Engine::default());
    let service = viewer_with_page(&engine, &root, files).await;
    let (started, mounting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    *engine.held_mount.lock().unwrap() = Some((started, held));

    // The first document's layout is mounting the viewer when the reload starts.
    let showing = tokio::spawn(super::for_window_generation(1, {
        let service = service.clone();
        async move { service.layout(Some(("chat".to_owned(), AREA))).await }
    }));
    mounting.await.unwrap();
    let reset = tokio::spawn({
        let service = service.clone();
        async move { super::reset(2, None, Some(&service), None).await }
    });
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    release.send(()).unwrap();
    showing.await.unwrap().unwrap();
    reset.await.unwrap();

    assert!(!service.view("chat").unwrap().shown);
    assert_eq!(
        engine.placements.lock().unwrap().last(),
        Some(&None),
        "the reset hides the viewer after its mount finished"
    );
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_viewer_mounting_across_a_reload_ends_hidden_in_the_project_tab() {
    viewer_mount_across_reset(false).await;
}

#[tokio::test]
async fn a_viewer_mounting_across_a_reload_ends_hidden_in_the_file_tab() {
    viewer_mount_across_reset(true).await;
}

#[tokio::test]
async fn a_menu_of_the_replaced_document_never_ends_the_new_documents_handoff() {
    let engine = Arc::new(Engine::default());
    let service = PreviewService::new(engine.clone(), PreviewConfig::new(PathBuf::new()));
    let page = service.create_web_tab("chat").unwrap();
    service.switch_tab("chat", &page).unwrap();
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
    super::reset(1, Some(&service), None, None).await;
    super::reset(2, Some(&service), None, None).await;
    super::for_window_generation(2, async {
        place_browser(&service, &page).await.unwrap();
        service
            .prepare_browser_overlay("chat", &page)
            .await
            .unwrap();
        service.show_browser_overlay("chat", &page).await.unwrap();
    })
    .await;
    let placements = engine.placements.lock().unwrap().len();

    super::for_window_generation(1, async {
        service.close_browser_overlay("chat", &page).await.unwrap();
        assert!(matches!(
            service.prepare_browser_overlay("chat", &page).await,
            Err(vw_preview::PreviewError::NotInView)
        ));
        service.show_browser_overlay("chat", &page).await.unwrap();
    })
    .await;

    assert_eq!(
        engine.overlays.lock().unwrap().as_slice(),
        &[true],
        "the new document's page stays parked under its menu"
    );
    assert_eq!(engine.placements.lock().unwrap().len(), placements);
    super::for_window_generation(2, service.close_browser_overlay("chat", &page))
        .await
        .unwrap();
    assert_eq!(engine.overlays.lock().unwrap().as_slice(), &[true, false]);
}
