//! Unlock-to-Send regressions using the parent model-list RPC fixture and isolated `KeyService`.
//! Previously served picks have no pending replay; their locked runtime must not retire them.
#![allow(clippy::unwrap_used)]

use super::*;

async fn ready_key(run: &Run, locked: bool) {
    let keys = run.core.shared.config.keys.as_ref().unwrap();
    if locked {
        keys.unlock(
            vw_vault::SecretString::new("refresh notice test vault"),
            None,
        )
        .await
        .unwrap();
    } else {
        keys.store_key(vw_vault::SecretString::new(
            "unlock-pick-test-key-000000000000000",
        ))
        .await
        .unwrap();
    }
    assert!(keys.speech_available());
}

async fn remembered_run(
    name: &str,
    locked: bool,
    effort: &str,
) -> (Run, super::super::super::Choice) {
    let (run, mut saved) = saved_catalog_run(name, locked).await;
    saved.effort = effort.into();
    run.core
        .shared
        .model_choices
        .records
        .update(CHAT, |record| {
            record.remembered = Some(saved.clone());
            record.pending = None;
        })
        .unwrap();
    (run, saved)
}

async fn attach_locked(run: &Run, saved: &super::super::super::Choice) -> Value {
    only_large_4(run, "local");
    let last = run.resumed().await;
    let restored =
        super::super::super::restore(&run.core.shared, CHAT, last.clone(), Some(saved.clone()))
            .await;
    assert_eq!(restored, last);
    super::super::super::publish(&run.core.shared, CHAT, restored.clone()).await;
    restored
}

#[tokio::test]
async fn unlock_send_replays_remembered_model_and_thinking() {
    for (locked, effort, method) in [
        (true, "off", "turn/start"),
        (false, "high", "turn/start"),
        (true, "off", "session/turn/enqueue"),
        (true, "high", "session/turn/queue/resume"),
    ] {
        let name = format!("unlock-remembered-{locked}-{effort}-{method}").replace('/', "-");
        let (run, saved) = remembered_run(&name, locked, effort).await;
        let before = run.saved();
        let mut events = run.core.subscribe(Some(CHAT.into()));
        attach_locked(&run, &saved).await;
        assert_eq!(run.saved(), before);
        ready_key(&run, locked).await;
        std::fs::write(run.home.join("models.json"), models().to_string()).unwrap();
        super::super::super::before(&run.core.shared, method, &json!({"sessionId":CHAT}))
            .await
            .unwrap();
        assert_eq!(
            active(&run.resumed().await),
            (saved.alias.clone(), effort.into())
        );
        assert_eq!(run.state()["writes"], json!([[saved.alias, effort]]));
        assert_eq!(
            run.saved(),
            before,
            "Send replaced a previously served choice"
        );
        assert_eq!(alerts(&mut events).await, Vec::<String>::new());
        run.finish().await;
    }
}

#[tokio::test]
async fn unlock_stale_turn_notification_cannot_retire_the_saved_pick() {
    let (run, saved) = remembered_run("unlock-stale-turn", true, "off").await;
    let shared = &run.core.shared;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":CHAT,"status":{"type":"idle"}},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    let before = run.saved();
    attach_locked(&run, &saved).await;
    ready_key(&run, true).await;
    std::fs::write(run.home.join("models.json"), models().to_string()).unwrap();
    let mut events = run.core.subscribe(Some(CHAT.into()));
    super::super::super::turn_started(shared, &json!({"sessionId":CHAT})).await;
    assert_eq!(
        run.saved(),
        before,
        "the old locked runtime retired the saved pick after unlock"
    );
    assert_eq!(run.state()["writes"], json!([]));
    assert_eq!(alerts(&mut events).await, Vec::<String>::new());
    run.finish().await;
}

#[tokio::test]
async fn unlock_same_model_restores_its_saved_thinking() {
    let (run, saved) = remembered_run("unlock-thinking", true, "off").await;
    let before = run.saved();
    attach_locked(&run, &saved).await;
    ready_key(&run, true).await;
    std::fs::write(run.home.join("models.json"), models().to_string()).unwrap();
    let mut state = run.state();
    state["active"] = json!(saved.alias);
    state["thinking"] = json!("high");
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    super::super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
        .await
        .unwrap();
    assert_eq!(active(&run.resumed().await), (saved.alias, "off".into()));
    assert_eq!(run.saved(), before);
    run.finish().await;
}

#[tokio::test]
async fn unlock_locked_reopen_keeps_offered_saved_thinking() {
    let (run, saved) = remembered_run("unlock-reopen-thinking", true, "off").await;
    let before = run.saved();
    let mut state = run.state();
    state["warnings"] = json!([]);
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    let current = run.resumed().await;
    assert_eq!(active(&current), (saved.alias.clone(), "high".into()));
    let pin = super::super::super::recorded(&json!({"model":saved.alias,"reasoningEffort":"high"}));
    let restored = super::super::super::restore(&run.core.shared, CHAT, current, pin).await;
    assert_eq!(active(&restored), (saved.alias, "off".into()));
    assert_eq!(run.saved(), before);
    run.finish().await;
}

#[tokio::test]
async fn unlock_incomplete_thinking_levels_cannot_retire_saved_effort() {
    let (run, saved) = remembered_run("unlock-incomplete-levels", true, "off").await;
    let before = run.saved();
    let mut rows = models();
    for row in rows.as_array_mut().unwrap() {
        row["source"] = json!("local");
        row["thinkingLevels"] = json!(["high"]);
    }
    std::fs::write(run.home.join("models.json"), rows.to_string()).unwrap();
    let current = run.resumed().await;
    let restored =
        super::super::super::restore(&run.core.shared, CHAT, current.clone(), Some(saved.clone()))
            .await;
    assert_eq!(restored, current);
    assert_eq!(run.saved(), before);
    ready_key(&run, true).await;
    std::fs::write(run.home.join("models.json"), models().to_string()).unwrap();
    super::super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
        .await
        .unwrap();
    assert_eq!(active(&run.resumed().await), (saved.alias, "off".into()));
    assert_eq!(run.saved(), before);
    run.finish().await;
}

#[tokio::test]
async fn unlock_really_retired_choice_keeps_fallback_notice() {
    let (run, saved) = remembered_run("unlock-really-retired", true, "off").await;
    attach_locked(&run, &saved).await;
    ready_key(&run, true).await;
    only_large_4(&run, "catalog");
    let mut events = run.core.subscribe(Some(CHAT.into()));
    super::super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
        .await
        .unwrap();
    assert_eq!(active(&run.resumed().await), large_4("high"));
    assert_eq!(run.saved()["remembered"]["alias"], "mistral-large-4-0");
    assert_eq!(alerts(&mut events).await.len(), 1);
    run.finish().await;
}

#[tokio::test]
async fn unlock_refused_send_retains_replay_until_key_setup() {
    let (run, saved) = remembered_run("unlock-before-key-setup", false, "off").await;
    only_large_4(&run, "local");
    let before = run.saved();
    let error =
        super::super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
            .await
            .unwrap_err();
    assert!(error.message.contains("add a Mistral key"));
    ready_key(&run, false).await;
    std::fs::write(run.home.join("models.json"), models().to_string()).unwrap();
    super::super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
        .await
        .unwrap();
    assert_eq!(active(&run.resumed().await), (saved.alias, "off".into()));
    assert_eq!(run.saved(), before);
    run.finish().await;
}

#[tokio::test]
async fn unlock_failed_replay_refuses_send_and_keeps_record() {
    let (run, saved) = remembered_run("unlock-write-fails", true, "off").await;
    attach_locked(&run, &saved).await;
    let before = run.saved();
    ready_key(&run, true).await;
    std::fs::write(run.home.join("models.json"), models().to_string()).unwrap();
    std::fs::write(run.home.join("write-fails"), "fail").unwrap();
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let error =
        super::super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
            .await
            .unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::InvalidParams);
    assert_eq!(run.saved(), before);
    assert_eq!(alerts(&mut events).await, Vec::<String>::new());
    std::fs::remove_file(run.home.join("write-fails")).unwrap();
    super::super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
        .await
        .unwrap();
    assert_eq!(active(&run.resumed().await), (saved.alias, "off".into()));
    assert_eq!(run.saved(), before);
    run.finish().await;
}

#[tokio::test]
async fn unlock_missing_send_session_leaves_saved_choices_untouched() {
    let (run, _) = remembered_run("unlock-missing-session", true, "off").await;
    let before = run.saved();
    for params in [Value::Null, json!({}), json!({"sessionId":null})] {
        super::super::super::before(&run.core.shared, "turn/start", &params)
            .await
            .unwrap();
    }
    assert_eq!(run.saved(), before);
    assert_eq!(run.state()["writes"], json!([]));
    run.finish().await;
}

#[tokio::test]
async fn unlock_changed_backend_still_requires_fallback_notice() {
    let (run, saved) = remembered_run("unlock-changed-backend", true, "off").await;
    attach_locked(&run, &saved).await;
    ready_key(&run, true).await;
    let mut rows = models();
    rows[1]["provider"] = json!("llamacpp");
    std::fs::write(run.home.join("models.json"), rows.to_string()).unwrap();
    let mut events = run.core.subscribe(Some(CHAT.into()));
    super::super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
        .await
        .unwrap();
    assert_eq!(active(&run.resumed().await), large_4("high"));
    assert_eq!(run.saved()["remembered"]["alias"], "mistral-large-4-0");
    assert_eq!(alerts(&mut events).await.len(), 1);
    run.finish().await;
}
