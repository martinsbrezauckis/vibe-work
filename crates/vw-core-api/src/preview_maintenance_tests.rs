//! Preview maintenance counts real process scans and timer rounds on an offline core.
//! Uses a windowless engine; verifies idle sleep, port withdrawal, wake-up after an open and the immediate round
//! after a chat server's exit notice.

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
    core_with(name, Arc::new(Engine)).await
}

async fn core_with(name: &str, engine: Arc<dyn PreviewEngine>) -> crate::Core {
    crate::offline_core::start_with(name, |config, dir| {
        config.preview = Some(super::PreviewSetup {
            engine,
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

/// Records every URL rule update it applies (the App preview's content rules on macOS).
#[derive(Default)]
struct RulesEngine {
    updates: std::sync::Mutex<Vec<vw_netguard::rules::RulePage>>,
}
impl PreviewEngine for RulesEngine {
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
    fn update_rules(
        &self,
        _: &MountId,
        rules: vw_netguard::rules::RulePage,
    ) -> EngineFuture<'_, ()> {
        self.updates.lock().unwrap().push(rules);
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

/// PREDICT: on macOS the App preview's content rules are the only control on loopback literals, and they followed a
/// stopped chat server only at the next port re-check (up to 3 s): another program that bound the vacated port in
/// that window was reachable. An exit notice for a chat server now runs the re-check at once, so its port leaves
/// the App preview's rules well before the periodic round (here: no listener on the port, so the round withdraws it).
#[tokio::test]
async fn a_chat_server_exit_notice_refreshes_the_app_preview_rules_at_once() {
    let engine = Arc::new(RulesEngine::default());
    let core = core_with("preview-exit-notice", engine.clone()).await;
    let hub = core.shared.preview.as_ref().unwrap();
    open(hub, "chat", vec![28107]).await;
    // Let the maintenance loop start its periodic wait (it woke for the open).
    tokio::time::sleep(Duration::from_millis(200)).await;
    let rounds = hub.maintenance_counts.rounds.load(Ordering::SeqCst);
    assert_eq!(rounds, 0, "no periodic round has run yet");
    let noticed = std::time::Instant::now();
    super::server_exited(&hub.exits, 4242);
    let withdrawn = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if engine
                .updates
                .lock()
                .unwrap()
                .iter()
                .any(|page| *page == vw_netguard::rules::RulePage::App { ports: Vec::new() })
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let elapsed = noticed.elapsed();
    core.shutdown().await;
    assert!(
        withdrawn.is_ok(),
        "the App preview's rules still allowed the port 2 s after the exit notice"
    );
    assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
}
