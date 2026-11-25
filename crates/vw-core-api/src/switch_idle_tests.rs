//! Background switch application regressions; mock RPCs exercise publication, send locks and idle/running state.
use super::*;

async fn applied(core: &Core, ids: &[&str]) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while ids.iter().any(|id| core.shared.channels.switch_stale(id)) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("idle channels must apply the published switch without another turn");
}

#[tokio::test]
async fn idle_off_and_on_publish_before_send_lock_then_reload_each_chat_once() {
    let core = start_with_keys("loopapply-publish", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    chat(&core, "one", 1, false, false, &calls, ok);
    chat(&core, "two", 2, false, true, &calls, ok);
    for enabled in [false, true] {
        let one = core.shared.plugin_refresh.lock("one").await;
        let two = core.shared.plugin_refresh.lock("two").await;
        let took = tokio::time::timeout(Duration::from_secs(4), switch(&core, enabled))
            .await
            .expect("publication must not wait for the background send locks");
        eprintln!(
            "LOOPAPPLY_PUBLICATION enabled={enabled} took_ms={}",
            took.as_millis()
        );
        assert!(
            took < Duration::from_secs(1),
            "publication enabled={enabled} took {took:?}"
        );
        assert_eq!(core.shared.developer_permissions.enabled(), enabled);
        assert_eq!(config_disabled(&core), !enabled);
        let before = count(&calls, None, "config/reload");
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(count(&calls, None, "config/reload"), before);
        drop(one);
        drop(two);
        applied(&core, &["one", "two"]).await;
        for id in ["one", "two"] {
            assert_eq!(
                count(&calls, Some(id), "config/reload"),
                if enabled { 2 } else { 1 }
            );
        }
        assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 0);
    }
    core.shutdown().await;
}

#[tokio::test]
async fn idle_apply_leaves_running_chat_until_its_turn_ends() {
    let core = start_with_keys("loopapply-busy", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    chat(&core, "busy", 1, true, true, &calls, ok);
    chat(&core, "idle", 2, false, false, &calls, ok);
    switch(&core, false).await;
    applied(&core, &["idle"]).await;
    assert_eq!(count(&calls, Some("busy"), "config/reload"), 0);
    assert!(core.shared.channels.switch_stale("busy"));
    let _ = core.shared.store.load(json!({
        "session": {"id": "busy", "cwd": core.shared.default_cwd, "status": {"type": "idle"}},
        "entries": [], "turns": [{"id": "t1"}]
    }), vw_session::LoadKind::New).unwrap();
    crate::switch_apply::after_turn(core.shared.clone(), "busy".to_owned()).await;
    assert_eq!(count(&calls, Some("busy"), "config/reload"), 1);
    assert!(!core.shared.channels.switch_stale("busy"));
    core.shutdown().await;
}

#[tokio::test]
async fn failed_idle_reload_preserves_never_ran_chat_and_stale_generation() {
    let core = start_with_keys("loopapply-failure", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    chat(&core, "new", 1, false, false, &calls, reload_fails);
    for (index, enabled) in [false, true].into_iter().enumerate() {
        switch(&core, enabled).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while count(&calls, Some("new"), "config/reload") < index + 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("background reload attempted");
        let held = core.shared.plugin_refresh.lock("new").await;
        assert!(core.shared.channels.has_channel("new"));
        assert!(core.shared.channels.switch_stale("new"));
        assert_eq!(count(&calls, Some("new"), "config/reload"), index + 1);
        assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 0);
        drop(held);
    }
    core.shutdown().await;
}

#[tokio::test]
async fn a_blocked_chat_does_not_delay_another_and_all_share_one_budget() {
    let core = start_with_keys("loopapply-budget", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    chat(&core, "blocked", 1, false, false, &calls, ok);
    chat(&core, "ready", 2, false, false, &calls, ok);
    let held = core.shared.plugin_refresh.lock("blocked").await;
    switch(&core, false).await;
    applied(&core, &["ready"]).await;
    tokio::time::pause();
    tokio::time::advance(core.shared.config.reopen_timeout + Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    drop(held);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(count(&calls, Some("blocked"), "config/reload"), 0);
    assert!(core.shared.channels.switch_stale("blocked"));
    let held = core.shared.plugin_refresh.lock("blocked").await;
    crate::switch_apply::before_work(&core.shared, "turn/start", &json!({"sessionId": "blocked"}))
        .await
        .unwrap();
    assert!(!core.shared.channels.switch_stale("blocked"));
    drop(held);
    core.shutdown().await;
}

#[tokio::test]
async fn empty_or_missing_command_config_needs_no_runtime_reload() {
    for (name, config) in [("missing", ""), ("empty", "mcp_servers = []\n")] {
        let core = start_with_keys(&format!("loopapply-{name}"), None, true, config).await;
        let calls: Calls = Arc::default();
        chat(&core, "new", 1, false, false, &calls, ok);
        switch(&core, false).await;
        assert!(!core.shared.developer_permissions.enabled());
        switch(&core, true).await;
        assert!(core.shared.developer_permissions.enabled());
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(sent_by_switch(&calls), 0);
        core.shutdown().await;
    }
}

#[tokio::test]
async fn a_chat_that_starts_running_while_the_worker_waits_is_not_reloaded() {
    let core = start_with_keys("loopapply-late-busy", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    chat(&core, "late", 1, false, true, &calls, ok);
    let held = core.shared.plugin_refresh.lock("late").await;
    switch(&core, false).await;
    let _ = core.shared.store.load(json!({
        "session": {"id": "late", "cwd": core.shared.default_cwd, "status": {"type": "running"}},
        "entries": [], "turns": [{"id": "t1"}]
    }), vw_session::LoadKind::New).unwrap();
    drop(held);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(count(&calls, Some("late"), "config/reload"), 0);
    assert!(core.shared.channels.switch_stale("late"));
    assert!(core.shared.channels.has_channel("late"));
    core.shutdown().await;
}

#[tokio::test]
async fn a_slow_reload_and_mutation_lock_wait_share_the_same_deadline() {
    let core = start_with_keys("loopapply-slow", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    for (number, id) in [(1, "one"), (2, "two")] {
        chat(&core, id, number, false, false, &calls, |method| {
            if method == "config/reload" {
                Answer::Slow(Duration::from_secs(120))
            } else {
                Answer::Ok
            }
        });
    }
    switch(&core, false).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while count(&calls, None, "config/reload") == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first reload sent");
    tokio::time::pause();
    tokio::time::advance(core.shared.config.reopen_timeout + Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    for id in ["one", "two"] {
        let held = core.shared.plugin_refresh.lock(id).await;
        assert!(core.shared.channels.switch_stale(id));
        assert!(core.shared.channels.has_channel(id));
        drop(held);
    }
    assert_eq!(
        count(&calls, None, "config/reload"),
        1,
        "one budget, not a fresh timeout per chat"
    );
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn failed_background_reload_keeps_saved_chats_and_their_background_processes() {
    let core = start_with_keys("loopapply-saved-failure", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    chat(&core, "saved", 1, false, true, &calls, reload_fails);
    chat(&core, "process", 2, false, true, &calls, reload_fails);
    let _ = core.shared.store.load(json!({
        "session": {"id": "process", "cwd": core.shared.default_cwd, "status": {"type": "idle"}},
        "entries": [], "turns": [{"id": "t1"}], "backgroundProcesses": [{"status": "running"}]
    }), vw_session::LoadKind::New).unwrap();
    switch(&core, false).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while count(&calls, None, "config/reload") < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("both reloads attempted");
    for id in ["saved", "process"] {
        let held = core.shared.plugin_refresh.lock(id).await;
        assert!(core.shared.channels.switch_stale(id));
        assert!(core.shared.channels.has_channel(id));
        drop(held);
    }
    core.shutdown().await;
}

#[tokio::test]
async fn a_later_switch_waits_for_the_prior_apply_budget_then_publishes() {
    let core = start_with_keys("loopapply-next-switch", None, true, ENABLED_ROW).await;
    switch(&core, false).await;
    let calls: Calls = Arc::default();
    chat(&core, "slow", 1, false, false, &calls, |method| {
        if method == "config/reload" {
            Answer::Slow(Duration::from_secs(120))
        } else {
            Answer::Ok
        }
    });
    switch(&core, true).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while count(&calls, None, "config/reload") == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("ON reload holds the mutation lock");
    let next = Core {
        shared: core.shared.clone(),
    };
    let off = tokio::spawn(async move { switch(&next, false).await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        !off.is_finished(),
        "a sent reload retains mutation ownership"
    );
    assert!(core.shared.developer_permissions.enabled());
    eprintln!("LOOPAPPLY_OVERLAP pending_off_while_on_reload=true");
    tokio::time::pause();
    tokio::time::advance(core.shared.config.reopen_timeout + Duration::from_secs(1)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(2), off)
        .await
        .unwrap()
        .unwrap();
    assert!(!core.shared.developer_permissions.enabled());
    assert!(config_disabled(&core));
    eprintln!("LOOPAPPLY_OVERLAP off_after_shared_budget=true");
    core.shutdown().await;
}
