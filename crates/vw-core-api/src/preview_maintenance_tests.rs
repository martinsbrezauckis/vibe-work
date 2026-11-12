//! Preview maintenance counts real process scans and timer rounds on an offline core.
//! Uses a windowless engine; verifies idle sleep, port withdrawal and wake-up after an open.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vw_preview::{
    ConsoleEntry, EngineFuture, HistoryStep, MountId, MountSpec, OpenRequest, Placement,
    PreviewEngine, PreviewHooks, Storage,
};

#[derive(Default)]
pub(super) struct Counts {
    pub(super) rounds: AtomicUsize,
    pub(super) scans: AtomicUsize,
}

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
        panic!("maintenance must not reload")
    }
    fn place(&self, _: &MountId, _: Option<Placement>) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn close(&self, _: &MountId) -> EngineFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn screenshot(&self, _: &MountId) -> EngineFuture<'_, Vec<u8>> {
        panic!("maintenance must not capture")
    }
    fn drain_console(&self, _: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn wipe(&self, _: &Storage) -> EngineFuture<'_, Vec<PathBuf>> {
        panic!("maintenance must preserve storage")
    }
}

async fn core(name: &str) -> crate::Core {
    crate::offline_core::start_with(name, |config, dir| {
        config.preview = Some(super::PreviewSetup {
            engine: Arc::new(Engine),
            config: vw_preview::PreviewConfig::new(dir.join("preview")),
            guard: None,
        });
    })
    .await
}

async fn open(hub: &super::PreviewHub, page: &str, ports: Vec<u16>) {
    hub.service
        .open(OpenRequest {
            session_id: page.to_owned(),
            url: if ports.is_empty() {
                "http://93.184.215.14/"
            } else {
                "http://127.0.0.1:28107/"
            }
            .parse()
            .unwrap(),
            allowed_ports: ports,
            downloads_dir: None,
            wait: false,
            lan: false,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn mounted_portless_web_tab_does_not_scan_processes() {
    let core = core("preview-portless-scan").await;
    let hub = core.shared.preview.as_ref().unwrap();
    let page = hub.service.create_web_tab("chat").unwrap();
    open(hub, &page, vec![]).await;
    assert_eq!(hub.service.mount_labels().len(), 1);
    assert_eq!(hub.service.sessions_with_ports(), Vec::<String>::new());
    tokio::time::sleep(super::PORTS_RECHECK + Duration::from_millis(200)).await;
    let scans = hub.maintenance_counts.scans.load(Ordering::SeqCst);
    core.shutdown().await;
    assert_eq!(scans, 0, "a portless web tab must not trigger an OS scan");
}

#[tokio::test]
async fn held_port_is_scanned_and_withdrawn_with_stopped_notice() {
    let core = core("preview-held-scan").await;
    let hub = core.shared.preview.as_ref().unwrap();
    open(hub, "chat", vec![28107]).await;
    tokio::time::timeout(Duration::from_secs(8), async {
        while hub.service.view("chat").unwrap().server.is_none() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert!(hub.maintenance_counts.scans.load(Ordering::SeqCst) > 0);
    // The stopped notice keeps re-checking even after all rights have been withdrawn.
    assert!(!hub.service.retain_allowed_ports("chat", &[]).await);
    assert_eq!(hub.service.sessions_with_ports(), vec!["chat"]);
    core.shutdown().await;
}

#[tokio::test]
async fn empty_preview_sleeps_until_an_open_then_returns_to_sleep() {
    let core = core("preview-idle-sleep").await;
    let hub = core.shared.preview.as_ref().unwrap();
    tokio::time::sleep(super::PORTS_RECHECK + Duration::from_millis(200)).await;
    assert_eq!(hub.maintenance_counts.rounds.load(Ordering::SeqCst), 0);
    open(hub, "chat", vec![28107]).await;
    tokio::time::timeout(Duration::from_secs(8), async {
        while hub.maintenance_counts.scans.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    hub.service.close("chat").await.unwrap();
    // Allow an in-progress round to finish, then there must be no recurring timer.
    tokio::time::sleep(super::PORTS_RECHECK + Duration::from_millis(200)).await;
    let rounds = hub.maintenance_counts.rounds.load(Ordering::SeqCst);
    tokio::time::sleep(super::PORTS_RECHECK + Duration::from_millis(200)).await;
    assert_eq!(hub.maintenance_counts.rounds.load(Ordering::SeqCst), rounds);
    core.shutdown().await;
}

#[tokio::test]
async fn portless_browser_still_reaps_after_ten_idle_minutes() {
    let core = core("preview-idle-reap").await;
    let hub = core.shared.preview.as_ref().unwrap();
    let page = hub.service.create_web_tab("chat").unwrap();
    open(hub, &page, vec![]).await;
    tokio::task::yield_now().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(601)).await;
    tokio::task::yield_now().await;
    assert_eq!(hub.service.mount_labels(), Vec::<String>::new());
    assert!(!hub.service.has_active_browsers());
    assert_eq!(hub.maintenance_counts.scans.load(Ordering::SeqCst), 0);
    core.shutdown().await;
}
