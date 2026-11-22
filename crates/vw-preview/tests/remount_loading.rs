//! A restored native view is loading before its first document event, including while guard canaries finish.
//! Uses a recording engine and explicit lifecycle callbacks; no browser, network request or wall-clock wait.
//! PREDICT: a restored entry still marked loaded skips the browser read's load wait and admits an empty
//! frame registry or a guard document; an early Finished must not complete the new document's load either.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use url::Url;
use vw_preview::{
    ConsoleEntry, EngineError, EngineFuture, GuardCheck, HistoryStep, MountId, MountSpec,
    OpenRequest, PageEvent, Placement, PreviewConfig, PreviewEngine, PreviewHooks, PreviewService,
    Rect, Storage,
};

#[derive(Default)]
struct DeferredEngine {
    hooks: OnceLock<Arc<dyn PreviewHooks>>,
    mounts: Mutex<Vec<MountSpec>>,
}

impl DeferredEngine {
    fn finish(&self, spec: &MountSpec) {
        let hooks = self.hooks.get().unwrap();
        assert!(hooks.page_event(&spec.id.label, PageEvent::Started(spec.url.clone())));
        assert!(hooks.page_event(&spec.id.label, PageEvent::Finished(spec.url.clone())));
    }
}

impl PreviewEngine for DeferredEngine {
    fn attach(&self, hooks: Arc<dyn PreviewHooks>) {
        assert!(self.hooks.set(hooks).is_ok());
    }
    fn mount(&self, spec: MountSpec) -> EngineFuture<'_, ()> {
        self.mounts.lock().unwrap().push(spec);
        Box::pin(async { Ok(()) })
    }
    fn navigate(&self, _: &MountId, _: Url) -> EngineFuture<'_, ()> {
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
        Box::pin(async { Err(EngineError::Unsupported("no pixels".into())) })
    }
    fn drain_console(&self, _: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn wipe(&self, _: &Storage) -> EngineFuture<'_, Vec<PathBuf>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn request(session: &str) -> OpenRequest {
    OpenRequest {
        session_id: session.into(),
        url: "https://93.184.215.14/".parse().unwrap(),
        allowed_ports: Vec::new(),
        downloads_dir: None,
        wait: false,
        lan: false,
    }
}

#[tokio::test]
async fn restored_view_waits_for_its_document_instead_of_reading_the_canary() {
    let engine = Arc::new(DeferredEngine::default());
    let mut config = PreviewConfig::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".scratch/remount-loading"),
    );
    config.live_cap = 1;
    let service = PreviewService::new(engine.clone(), config);
    service.set_engine_pool("1").await.unwrap();
    // Complete the original document, then let another chat evict its hidden native view.
    service.open(request("first")).await.unwrap();
    let original = engine.mounts.lock().unwrap().last().unwrap().clone();
    engine.finish(&original);
    assert!(!service.load_state("first").unwrap().loading);
    service.open(request("second")).await.unwrap();
    let second = engine.mounts.lock().unwrap().last().unwrap().clone();
    engine.finish(&second);
    assert!(service.page_mount("first").is_none());
    let changes = service.load_changes("first").unwrap();
    service
        .report_tab_layout(
            Some("first".into()),
            Some(Rect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            }),
        )
        .await
        .unwrap();
    let restored = engine.mounts.lock().unwrap().last().unwrap().clone();
    assert_ne!(restored.id, original.id);
    assert!(service.page_mount("first").is_some());
    assert!(
        service.load_state("first").unwrap().loading,
        "restored native view falsely reports loaded before its first document event"
    );
    assert!(
        changes.has_changed().unwrap(),
        "load waiters were not notified"
    );
    let hooks = engine.hooks.get().unwrap();
    for canary in [
        "http://probe.vw-guard.invalid/",
        "http://127.0.0.1:43210/__vw_guard/probe",
    ] {
        let canary: Url = canary.parse().unwrap();
        hooks.network_guard(&restored.id.label, GuardCheck::Expect(canary.clone()));
        hooks.network_guard(&restored.id.label, GuardCheck::Done(true));
        assert!(hooks.page_event(&restored.id.label, PageEvent::Finished(canary)));
        assert!(
            service.load_state("first").unwrap().loading,
            "a late canary completion ended the document wait"
        );
    }
    // WebKit can finish the cancelled canary under the next pending URI before that document starts.
    assert!(hooks.page_event(
        &restored.id.label,
        PageEvent::Finished(restored.url.clone())
    ));
    assert!(
        service.load_state("first").unwrap().loading,
        "a completion before the target document started ended the restored load"
    );
    assert!(!service.load_state("second").unwrap().loading);
    engine.finish(&restored);
    assert!(!service.load_state("first").unwrap().loading);
}
