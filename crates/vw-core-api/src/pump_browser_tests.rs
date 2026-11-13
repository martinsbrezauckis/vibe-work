//! Browser activity across snapshots and questions. Uses the real pump, offline core and recording helper;
//! no native page operation is permitted, and release tasks are drained before checking their effects.
use super::*;
use std::path::PathBuf;
use vw_preview::{
    ConsoleEntry, EngineFuture, HistoryStep, MountId, MountSpec, Placement, PreviewEngine,
    PreviewHooks, Storage,
};

struct NoPageCalls;
impl PreviewEngine for NoPageCalls {
    fn attach(&self, _: Arc<dyn PreviewHooks>) {}
    fn mount(&self, _: MountSpec) -> EngineFuture<'_, ()> {
        panic!("activity tracking must not mount a page")
    }
    fn navigate(&self, _: &MountId, _: url::Url) -> EngineFuture<'_, ()> {
        panic!("activity tracking must not navigate")
    }
    fn history(&self, _: &MountId, _: HistoryStep) -> EngineFuture<'_, ()> {
        panic!("activity tracking must not reload")
    }
    fn place(&self, _: &MountId, _: Option<Placement>) -> EngineFuture<'_, ()> {
        panic!("activity tracking must not move a page")
    }
    fn close(&self, _: &MountId) -> EngineFuture<'_, ()> {
        panic!("activity tracking must not close a page")
    }
    fn screenshot(&self, _: &MountId) -> EngineFuture<'_, Vec<u8>> {
        panic!("activity tracking must not capture a page")
    }
    fn drain_console(&self, _: &MountId) -> EngineFuture<'_, Vec<ConsoleEntry>> {
        panic!("activity tracking must not read a page")
    }
    fn wipe(&self, _: &Storage) -> EngineFuture<'_, Vec<PathBuf>> {
        panic!("activity tracking must preserve site data")
    }
}

fn pump(shared: &Arc<Shared>) -> Pump {
    Pump {
        weak: Arc::downgrade(shared),
        generation: 0,
        items: mpsc::channel(1).0,
        resyncs: JoinSet::new(),
        browser_releases: JoinSet::new(),
        later: BTreeMap::new(),
        in_flight: BTreeSet::new(),
        queued: BTreeSet::new(),
        open_callbacks: BTreeMap::new(),
        budget: ResyncBudget::standard(),
    }
}

async fn snapshot_activity(name: &str, turns: Value, expected_active: bool) {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.preview = Some(crate::PreviewSetup {
            engine: Arc::new(NoPageCalls),
            config: vw_preview::PreviewConfig::new(dir.join("preview")),
            guard: None,
        });
    })
    .await;
    let shared = &core.shared;
    let initial = shared
        .store
        .load(
            json!({"session":{"id":"chat","status":{"type":"idle"}},"history":[],"turns":turns}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    let hub = shared.preview.as_ref().unwrap();
    let page = hub.service.create_web_tab("chat").unwrap();
    let other = hub.service.create_web_tab("other").unwrap();
    drop(
        hub.browser
            .sessions()
            .begin(&page, vw_browser::Actor::Agent),
    );
    assert!(hub.browser.sessions().state(&page).agent_active);
    drop(
        hub.browser
            .sessions()
            .begin(&other, vw_browser::Actor::Agent),
    );

    // The session/start snapshot was queued before the action; process it only after that action finishes.
    let mut pump = pump(shared);
    pump.after_apply(shared, &initial);
    while let Some(released) = pump.browser_releases.join_next().await {
        released.unwrap();
    }
    let state = hub.browser.sessions().state(&page);
    let other_state = hub.browser.sessions().state(&other);
    core.shutdown().await;
    assert_eq!(
        state.agent_active, expected_active,
        "snapshot activity: {state:?}"
    );
    assert!(
        other_state.agent_active,
        "another chat's browser must keep its activity"
    );
}

#[tokio::test]
async fn delayed_initial_idle_snapshot_does_not_end_new_browser_activity() {
    snapshot_activity("initial-browser-activity", json!([]), true).await;
}

#[tokio::test]
async fn terminal_snapshots_release_only_their_chats_browser_activity() {
    for status in ["completed", "failed", "interrupted"] {
        snapshot_activity(
            &format!("terminal-browser-{status}"),
            json!([{"id":"turn", "status":status}]),
            false,
        )
        .await;
    }
    snapshot_activity(
        "active-browser-snapshot",
        json!([{"id":"old","status":"completed"},{"id":"current","status":"in_progress"}]),
        true,
    )
    .await;
}

#[tokio::test]
async fn delayed_initial_snapshot_keeps_connected_browser_sessions_active() {
    use crate::chrome_control::{commands, gateway, test_support};
    let core = crate::offline_core::start_with("initial-connected-browser", |config, dir| {
        config.chrome_control.package = Some(dir.join("chrome-package"));
    })
    .await;
    let shared = &core.shared;
    test_support::answering(shared, |_, args| {
        json!({"structuredContent":{"sessionId":args["profileId"],
            "profileId":args["profileId"],"state":"active"}})
    });
    std::fs::write(
        shared.chrome_control.dir().unwrap().join("setup.json"),
        json!({"files":[],"browsers":["Google Chrome","Microsoft Edge"],"userData":[]}).to_string(),
    )
    .unwrap();
    test_support::observe_status(
        shared,
        &json!({"profiles":[
            {"profileId":"chrome:profile","connected":true,"extensionVersion":"0.1.3"},
            {"profileId":"edge:profile","connected":true,"extensionVersion":"0.1.3"}
        ]}),
    );
    let mut pump = pump(shared);
    for profile in ["chrome:profile"]
        .into_iter()
        .chain(cfg!(feature = "your-edge").then_some("edge:profile"))
    {
        let initial = shared
            .store
            .load(
                json!({"session":{"id":profile},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        shared.access.observe_request(
            "session/agent/update",
            profile,
            &json!({"agentName":"auto-approve"}),
        );
        gateway::call(
            shared,
            profile,
            "chrome_session",
            json!({"action":"open","profileId":profile}),
        )
        .await
        .unwrap();
        assert_eq!(
            commands::view(shared)
                .work
                .iter()
                .find(|row| row.session_id == profile)
                .unwrap()
                .state,
            "active"
        );
        pump.after_apply(shared, &initial);
        assert_eq!(
            commands::view(shared)
                .work
                .iter()
                .find(|row| row.session_id == profile)
                .unwrap()
                .state,
            "active",
            "the initial snapshot must not release {profile}"
        );
    }
    core.shutdown().await;
}

#[cfg(feature = "your-firefox")]
#[tokio::test]
async fn a_visible_question_has_released_the_browser_before_firefox_availability_finishes() {
    let core = crate::offline_core::start_with("question-before-browser-probe", |config, dir| {
        config.preview = Some(crate::PreviewSetup {
            engine: Arc::new(NoPageCalls),
            config: vw_preview::PreviewConfig::new(dir.join("preview")),
            guard: None,
        });
    })
    .await;
    let shared = &core.shared;
    let hub = shared.preview.as_ref().unwrap();
    let page = hub.service.create_web_tab("chat").unwrap();
    let other = hub.service.create_web_tab("other").unwrap();
    for page in [&page, &other] {
        drop(hub.browser.sessions().begin(page, vw_browser::Actor::Agent));
    }
    let (locked, ready) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let blocking = {
        let shared = shared.clone();
        std::thread::spawn(move || {
            crate::chrome_control::test_support::with_firefox_probe_held(&shared, || {
                locked.send(()).unwrap();
                held.recv().unwrap();
            });
        })
    };
    ready.await.unwrap();
    let mut events = shared.events.subscribe();
    let opening = {
        let shared = shared.clone();
        tokio::task::spawn_blocking(move || {
            crate::ask::cards::callback_opened(
                &shared,
                &json!({"id":"question", "sessionId":"chat", "detail":{"kind":"user_input"}}),
                "user_input",
                "chat",
            );
        })
    };
    let visible = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let CoreEvent::AskOpened { ask } = events.recv().await.unwrap()
                && ask.kind == "user_input"
            {
                break;
            }
        }
    })
    .await;
    let state = hub.browser.sessions().state(&page);
    let other_state = hub.browser.sessions().state(&other);
    let held = crate::browser_handover::check(shared, "chat");
    release.send(()).unwrap();
    blocking.join().unwrap();
    opening.await.unwrap();
    core.shutdown().await;
    assert!(
        visible.is_ok(),
        "question publication waited for Firefox availability"
    );
    assert!(
        !state.agent_active,
        "the visible question still leaves agent control: {state:?}"
    );
    assert!(other_state.agent_active);
    assert!(held.unwrap_err().contains("waiting for the user's answer"));
}
