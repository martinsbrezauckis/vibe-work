//! Developer-switch regression tests for fast publication, no forced discovery, and before-work recovery.
//! Before-work tests hold the same send lock as dispatch so background apply cannot race their direct calls.
//! The idle submodule separately tests background convergence. Work in progress is never cut.
//! Uses mock channels (one per chat) that record every request and can answer slowly or with an error.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::refresh_notice_tests::{drain_events, start_with_keys, tools_alerts};
use crate::{Command, Core, Origin};

#[path = "switch_addrefresh_tests.rs"]
mod addrefresh;

type Calls = Arc<Mutex<Vec<(String, String, Value)>>>;

const ENABLED_ROW: &str =
    "mcp_servers = [{ name = \"local\", transport = \"stdio\", command = \"/bin/echo\" }]\n";

/// How a mock chat answers one method.
#[derive(Clone, Copy)]
enum Answer {
    Ok,
    Slow(Duration),
    Error,
}

/// A chat with an open mock channel: `running` = a turn in progress, `ran` = it has a turn in its history.
fn chat(
    core: &Core,
    sid: &str,
    channel: u64,
    running: bool,
    ran: bool,
    calls: &Calls,
    answer: fn(&str) -> Answer,
) {
    let turns = if ran {
        json!([{"id": "t1"}])
    } else {
        json!([])
    };
    let status = if running { "running" } else { "idle" };
    let _ = core.shared.store.load(
        json!({
            "session": {"id": sid, "cwd": core.shared.default_cwd, "status": {"type": status}},
            "entries": [],
            "turns": turns
        }),
        vw_session::LoadKind::New,
    );
    let (client, server) = tokio::io::duplex(1 << 16);
    let (read, write) = tokio::io::split(client);
    let conn =
        vw_rpc::ConnectionBuilder::new(vw_rpc::ConnectionConfig::default()).start(read, write);
    core.shared.channels.bind_test_peer(sid, conn, (0, channel));
    let (sid, calls) = (sid.to_owned(), calls.clone());
    tokio::spawn(async move {
        let (read, write) = tokio::io::split(server);
        let write = Arc::new(tokio::sync::Mutex::new(write));
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let msg: Value = serde_json::from_str(&line).unwrap();
            let method = msg["method"].as_str().unwrap_or_default().to_owned();
            calls
                .lock()
                .unwrap()
                .push((sid.clone(), method.clone(), msg["params"].clone()));
            let Some(id) = msg.get("id").cloned() else {
                continue;
            };
            let write = write.clone();
            tokio::spawn(async move {
                let reply = match answer(&method) {
                    Answer::Ok => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
                    Answer::Slow(wait) => {
                        tokio::time::sleep(wait).await;
                        json!({"jsonrpc": "2.0", "id": id, "result": {}})
                    }
                    Answer::Error => {
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "refused"}})
                    }
                };
                let _ = write
                    .lock()
                    .await
                    .write_all(format!("{reply}\n").as_bytes())
                    .await;
            });
        }
    });
}

fn count(calls: &Calls, sid: Option<&str>, method: &str) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(s, m, _)| m == method && sid.is_none_or(|sid| s == sid))
        .count()
}

/// Requests that must wait behind the chat send lock while the switch publishes.
fn sent_by_switch(calls: &Calls) -> usize {
    count(calls, None, "mcp_catalog/refresh") + count(calls, None, "config/reload")
}

/// Longer than any switch time bound below: a switch that waited on one refresh fails the bound.
fn slow_refresh(method: &str) -> Answer {
    if method == "mcp_catalog/refresh" {
        Answer::Slow(Duration::from_secs(5))
    } else {
        Answer::Ok
    }
}

fn ok(_: &str) -> Answer {
    Answer::Ok
}

fn reload_fails(method: &str) -> Answer {
    if method == "config/reload" {
        Answer::Error
    } else {
        Answer::Ok
    }
}

async fn switch(core: &Core, enabled: bool) -> Duration {
    let started = Instant::now();
    core.execute(
        Origin::Ui,
        Command::DeveloperPermissions {
            enabled: Some(enabled),
        },
    )
    .await
    .unwrap();
    started.elapsed()
}

async fn try_switch(core: &Core, enabled: bool) -> bool {
    core.execute(
        Origin::Ui,
        Command::DeveloperPermissions {
            enabled: Some(enabled),
        },
    )
    .await
    .is_ok()
}

fn config_disabled(core: &Core) -> bool {
    let text =
        std::fs::read_to_string(core.shared.instructions.own_home.join("config.toml")).unwrap();
    text.contains("disabled = true")
}

#[tokio::test]
async fn off_with_idle_open_chats_returns_without_refreshing_them() {
    let core = start_with_keys("devlazy-off-idle", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    let _send0 = core.shared.plugin_refresh.lock("i1").await;
    let _send1 = core.shared.plugin_refresh.lock("i2").await;
    let _send2 = core.shared.plugin_refresh.lock("i3").await;
    for (n, sid) in ["i1", "i2", "i3"].into_iter().enumerate() {
        chat(
            &core,
            sid,
            u64::try_from(n + 1).unwrap(),
            false,
            true,
            &calls,
            slow_refresh,
        );
    }
    let took = switch(&core, false).await;
    assert!(
        config_disabled(&core),
        "the command server is paused on disk"
    );
    assert!(!core.shared.developer_permissions.enabled());
    assert_eq!(
        sent_by_switch(&calls),
        0,
        "publication does not wait for idle chats holding their send locks"
    );
    assert!(took < Duration::from_secs(4), "OFF returned in {took:?}");
    for sid in ["i1", "i2", "i3"] {
        assert!(
            core.shared.channels.switch_stale(sid),
            "{sid} applies the change before its next work"
        );
    }
    core.shutdown().await;
}

#[tokio::test]
async fn off_sends_a_running_chat_nothing_and_lets_its_turn_finish() {
    let core = start_with_keys("devlazy-off-running", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    let _send0 = core.shared.plugin_refresh.lock("busy").await;
    let _send1 = core.shared.plugin_refresh.lock("idle").await;
    chat(&core, "busy", 1, true, true, &calls, slow_refresh);
    chat(&core, "idle", 2, false, true, &calls, slow_refresh);
    let took = switch(&core, false).await;
    assert!(took < Duration::from_secs(4), "OFF returned in {took:?}");
    assert_eq!(sent_by_switch(&calls), 0, "no forced refresh anywhere");
    assert!(
        core.shared.channels.has_channel("busy"),
        "its turn is not cut"
    );
    assert!(core.shared.channels.switch_stale("busy"));
    assert!(core.shared.channels.switch_stale("idle"));
    core.shutdown().await;
}

#[tokio::test]
async fn a_new_idle_chat_reloads_once_before_its_first_work_and_then_sends_normally() {
    let core = start_with_keys("devlazy-next-new", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    let _send0 = core.shared.plugin_refresh.lock("fresh").await;
    chat(&core, "fresh", 1, false, false, &calls, ok);
    switch(&core, false).await;
    let params = json!({"sessionId": "fresh"});
    crate::switch_apply::before_work(&core.shared, "turn/start", &params)
        .await
        .unwrap();
    assert_eq!(count(&calls, Some("fresh"), "config/reload"), 1);
    assert_eq!(
        count(&calls, Some("fresh"), "mcp_catalog/refresh"),
        0,
        "no forced discovery while OFF"
    );
    assert!(!core.shared.channels.switch_stale("fresh"));
    // Applied once: the next work sends nothing extra; a read never applies anything.
    crate::switch_apply::before_work(&core.shared, "turn/start", &params)
        .await
        .unwrap();
    crate::switch_apply::before_work(&core.shared, "session/read", &params)
        .await
        .unwrap();
    assert_eq!(count(&calls, Some("fresh"), "config/reload"), 1);
    core.shutdown().await;
}

#[tokio::test]
async fn an_idle_chat_that_ran_reloads_its_config_before_its_next_work_and_keeps_its_channel() {
    let core = start_with_keys("devlazy-next-ran", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    let _send0 = core.shared.plugin_refresh.lock("old").await;
    chat(&core, "old", 1, false, true, &calls, ok);
    switch(&core, false).await;
    crate::switch_apply::before_work(
        &core.shared,
        "session/turn/enqueue",
        &json!({"sessionId": "old"}),
    )
    .await
    .unwrap();
    assert_eq!(count(&calls, Some("old"), "config/reload"), 1);
    assert_eq!(count(&calls, Some("old"), "mcp_catalog/refresh"), 0);
    assert!(core.shared.channels.has_channel("old"));
    assert!(!core.shared.channels.switch_stale("old"));
    assert_eq!(count(&calls, Some("old"), "turn/start"), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn a_chat_that_ran_and_cannot_reload_is_closed_so_the_same_send_reopens_it_from_the_config() {
    let core = start_with_keys("devlazy-next-close", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    let _send0 = core.shared.plugin_refresh.lock("old").await;
    chat(&core, "old", 1, false, true, &calls, reload_fails);
    switch(&core, false).await;
    crate::switch_apply::before_work(&core.shared, "turn/start", &json!({"sessionId": "old"}))
        .await
        .unwrap();
    assert_eq!(count(&calls, Some("old"), "config/reload"), 1);
    assert!(
        !core.shared.channels.has_channel("old"),
        "the runtime that knew the command server is gone"
    );
    assert!(!core.shared.channels.switch_stale("old"));
    core.shutdown().await;
}

#[tokio::test]
async fn a_stale_chat_that_cannot_apply_the_change_is_refused_with_the_reason() {
    let core = start_with_keys("devlazy-refused", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    let _send0 = core.shared.plugin_refresh.lock("new-fails").await;
    let _send1 = core.shared.plugin_refresh.lock("busy-late").await;
    chat(&core, "new-fails", 1, false, false, &calls, reload_fails);
    chat(&core, "busy-late", 2, false, true, &calls, ok);
    switch(&core, false).await;
    // A turn started in the second chat after the switch (its channel is still the old one).
    let _ = core.shared.store.load(
        json!({"session": {"id": "busy-late", "cwd": core.shared.default_cwd, "status": {"type": "running"}}, "entries": [], "turns": [{"id": "t1"}]}),
        vw_session::LoadKind::New,
    );
    let mut sub = core.subscribe(None);
    let _ = drain_events(&mut sub).await;
    for sid in ["new-fails", "busy-late"] {
        let error = crate::switch_apply::before_work(
            &core.shared,
            "turn/start",
            &json!({"sessionId": sid}),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.server_code.as_deref(),
            Some("developerSwitchNotApplied"),
            "{sid}"
        );
        assert_eq!(
            error.message,
            vw_vocabulary::texts::DEVELOPER_SWITCH_NOT_APPLIED
        );
        assert!(
            core.shared.channels.switch_stale(sid),
            "{sid} tries again next time"
        );
    }
    let alerts = tools_alerts(&drain_events(&mut sub).await);
    assert_eq!(alerts.len(), 2, "{alerts:?}");
    assert_eq!(
        count(&calls, Some("busy-late"), "config/reload"),
        0,
        "a working chat is not reloaded"
    );
    assert!(
        core.shared.channels.has_channel("busy-late"),
        "a working chat is never cut"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn on_is_symmetric_no_chat_is_refreshed_and_every_open_chat_applies_it_later() {
    let core = start_with_keys("devlazy-on", None, true, ENABLED_ROW).await;
    switch(&core, false).await;
    assert!(config_disabled(&core));
    let calls: Calls = Arc::default();
    let _send0 = core.shared.plugin_refresh.lock("busy").await;
    let _send1 = core.shared.plugin_refresh.lock("i1").await;
    let _send2 = core.shared.plugin_refresh.lock("i2").await;
    chat(&core, "busy", 1, true, true, &calls, slow_refresh);
    for (n, sid) in ["i1", "i2"].into_iter().enumerate() {
        chat(
            &core,
            sid,
            u64::try_from(n + 2).unwrap(),
            false,
            true,
            &calls,
            slow_refresh,
        );
    }
    let took = switch(&core, true).await;
    assert!(!config_disabled(&core), "resumed on disk");
    assert!(took < Duration::from_secs(4), "ON returned in {took:?}");
    assert_eq!(sent_by_switch(&calls), 0);
    for sid in ["busy", "i1", "i2"] {
        assert!(core.shared.channels.switch_stale(sid), "{sid}");
    }
    crate::switch_apply::before_work(&core.shared, "turn/start", &json!({"sessionId": "i1"}))
        .await
        .unwrap();
    assert_eq!(count(&calls, Some("i1"), "config/reload"), 1);
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn a_stale_chat_applies_the_change_when_its_turn_ended_and_a_busy_one_waits() {
    let core = start_with_keys("devlazy-after-turn", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    chat(&core, "done", 1, false, true, &calls, ok);
    chat(&core, "queued", 2, true, true, &calls, ok);
    switch(&core, false).await;
    crate::switch_apply::after_turn(core.shared.clone(), "done".to_owned()).await;
    crate::switch_apply::after_turn(core.shared.clone(), "queued".to_owned()).await;
    assert_eq!(count(&calls, Some("done"), "config/reload"), 1);
    assert!(!core.shared.channels.switch_stale("done"));
    assert_eq!(count(&calls, Some("queued"), "config/reload"), 0);
    assert!(core.shared.channels.switch_stale("queued"));
    // Applied: a later end of turn sends nothing more.
    crate::switch_apply::after_turn(core.shared.clone(), "done".to_owned()).await;
    assert_eq!(count(&calls, Some("done"), "config/reload"), 1);
    core.shutdown().await;
}

#[tokio::test]
async fn an_open_that_raced_the_switch_counts_as_stale() {
    let core = start_with_keys("devlazy-race-open", None, true, ENABLED_ROW).await;
    let before = core.shared.channels.switch_generation();
    switch(&core, false).await;
    assert_eq!(core.shared.channels.switch_generation(), before + 1);
    let calls: Calls = Arc::default();
    chat(&core, "after", 1, false, true, &calls, ok);
    assert!(
        !core.shared.channels.switch_stale("after"),
        "opened after the switch: follows it"
    );
    // An open whose request was sent before the bump records the older generation.
    core.shared.channels.bump_switch_generation();
    assert!(core.shared.channels.switch_stale("after"));
    core.shutdown().await;
}

#[tokio::test]
async fn concurrent_switches_and_before_work_finish_and_leave_no_command_server_enabled_while_off()
{
    let core = start_with_keys("devlazy-concurrent", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    chat(&core, "fresh", 1, false, false, &calls, ok);
    chat(&core, "busy", 2, true, true, &calls, ok);
    let fresh = json!({"sessionId": "fresh"});
    let all = async {
        for _ in 0..3 {
            // An ON whose confirmation an OFF overtook is refused (`developerPermissionsChanged`); any
            // outcome is fine here as long as everything finishes and the last OFF wins.
            let _ = tokio::join!(
                try_switch(&core, false),
                crate::switch_apply::before_work(&core.shared, "turn/start", &fresh),
                try_switch(&core, true),
            );
        }
        switch(&core, false).await;
    };
    tokio::time::timeout(Duration::from_secs(60), all)
        .await
        .expect("no deadlock with the mutation lock");
    assert!(!core.shared.developer_permissions.enabled());
    assert!(
        config_disabled(&core),
        "OFF last: the command server is paused on disk"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn on_apply_enables_only_resumed_rows_in_ran_and_never_ran_chats() {
    let config = "mcp_servers = [{ name = \"local\", transport = \"stdio\", command = \"/bin/echo\" }, { name = \"person-disabled\", transport = \"stdio\", command = \"/bin/echo\", disabled = true }]\n";
    let core = start_with_keys("devlazy-on-targeted", None, true, config).await;
    let calls: Calls = Arc::default();
    let _send0 = core.shared.plugin_refresh.lock("ran").await;
    let _send1 = core.shared.plugin_refresh.lock("never-ran").await;
    chat(&core, "ran", 1, false, true, &calls, ok);
    chat(&core, "never-ran", 2, false, false, &calls, ok);
    switch(&core, false).await;
    for id in ["ran", "never-ran"] {
        crate::switch_apply::before_work(&core.shared, "turn/start", &json!({"sessionId": id}))
            .await
            .unwrap();
    }
    assert_eq!(
        count(&calls, None, "mcp_catalog/toggle"),
        0,
        "no enable while OFF"
    );
    switch(&core, true).await;
    assert_eq!(
        count(&calls, None, "mcp_catalog/toggle"),
        0,
        "nothing sent at ON"
    );
    for id in ["ran", "never-ran"] {
        crate::switch_apply::before_work(&core.shared, "turn/start", &json!({"sessionId": id}))
            .await
            .unwrap();
        crate::switch_apply::before_work(&core.shared, "turn/start", &json!({"sessionId": id}))
            .await
            .unwrap();
        let calls = calls.lock().unwrap();
        let toggles: Vec<_> = calls
            .iter()
            .filter(|(sid, method, _)| sid == id && method == "mcp_catalog/toggle")
            .collect();
        assert_eq!(toggles.len(), 1, "one targeted enable for {id}: {calls:?}");
        assert_eq!(toggles[0].2["name"], "local");
        assert_eq!(toggles[0].2["source"], "server");
        assert_eq!(toggles[0].2["disabled"], false);
    }
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn on_apply_failure_keeps_a_never_ran_chat_stale_and_refuses_work() {
    let core = start_with_keys("devlazy-on-failed", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    let _send0 = core.shared.plugin_refresh.lock("new").await;
    chat(&core, "new", 1, false, false, &calls, |method| {
        if method == "mcp_catalog/toggle" {
            Answer::Error
        } else {
            Answer::Ok
        }
    });
    switch(&core, false).await;
    switch(&core, true).await;
    let error =
        crate::switch_apply::before_work(&core.shared, "turn/start", &json!({"sessionId": "new"}))
            .await
            .unwrap_err();
    assert_eq!(
        error.server_code.as_deref(),
        Some("developerSwitchNotApplied")
    );
    assert!(core.shared.channels.switch_stale("new"));
    assert_eq!(count(&calls, None, "turn/start"), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn on_apply_does_not_reenable_a_row_the_person_disabled_after_on() {
    let core = start_with_keys("devlazy-on-person-off", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    let _send0 = core.shared.plugin_refresh.lock("new").await;
    chat(&core, "new", 1, false, false, &calls, ok);
    switch(&core, false).await;
    switch(&core, true).await;
    crate::command_servers_config::update_disabled(&core.shared.instructions.own_home, |_, _| {
        Some(true)
    })
    .unwrap();
    crate::switch_apply::before_work(&core.shared, "turn/start", &json!({"sessionId": "new"}))
        .await
        .unwrap();
    assert_eq!(count(&calls, None, "mcp_catalog/toggle"), 0);
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn on_apply_failure_closes_a_saved_chat_for_verified_reattach() {
    let core = start_with_keys("devlazy-on-saved-failed", None, true, ENABLED_ROW).await;
    let calls: Calls = Arc::default();
    let _send0 = core.shared.plugin_refresh.lock("saved").await;
    chat(&core, "saved", 1, false, true, &calls, |method| {
        if method == "mcp_catalog/toggle" {
            Answer::Error
        } else {
            Answer::Ok
        }
    });
    switch(&core, false).await;
    switch(&core, true).await;
    crate::switch_apply::before_work(&core.shared, "turn/start", &json!({"sessionId": "saved"}))
        .await
        .unwrap();
    assert!(!core.shared.channels.has_channel("saved"));
    assert_eq!(count(&calls, None, "mcp_catalog/toggle"), 1);
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 0);
    core.shutdown().await;
}

#[path = "switch_idle_tests.rs"]
mod idle;
