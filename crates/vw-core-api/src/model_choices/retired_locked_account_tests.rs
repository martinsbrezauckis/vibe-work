//! A locked or unset account key makes an absent catalog row inconclusive for a saved chat.
//! The parent fixture runs an isolated key service and a Python stdio model-list stand-in.
#![allow(clippy::unwrap_used)]

use super::*;

#[path = "retired_unlock_tests.rs"]
mod unlock_tests;

async fn saved_catalog_run(name: &str, locked: bool) -> (Run, super::super::Choice) {
    let run = Box::pin(start_with_key_state(
        name,
        &json!({}),
        "mistral-medium-latest",
        None,
        false,
        Some(locked),
    ))
    .await;
    let saved = super::super::identified(
        &run.core.shared,
        CHAT,
        super::super::Choice {
            alias: "mistral-medium-latest".into(),
            name: "mistral-medium-latest".into(),
            display_name: "Mistral Medium 3.5".into(),
            effort: "high".into(),
            provider: String::new(),
            provider_key: String::new(),
        },
    )
    .await;
    assert_eq!(saved.provider, "mistral");
    assert!(
        !saved.provider_key.is_empty(),
        "the saved catalog identity needs a bound backend"
    );
    run.core
        .shared
        .model_choices
        .records
        .update(CHAT, |record| {
            record.accepted(saved.clone());
        })
        .unwrap();
    (run, saved)
}

fn only_large_4(run: &Run, source: &str) {
    // A locked/no-key Vibe reports only its built-in model. Its missing catalog is not a
    // retirement decision; the chat's saved choice must remain for after unlock.
    let mut local = models()[0].clone();
    local["source"] = json!(source);
    std::fs::write(run.home.join("models.json"), json!([local]).to_string()).unwrap();
    let mut state = run.state();
    state["active"] = json!("mistral-large-4-0");
    state["default"] = json!("mistral-large-4-0");
    state["warnings"] = json!([]);
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
}

async fn unavailable_account_choice(locked: bool, send: bool) {
    let name = format!(
        "account-{}-{}",
        if locked { "locked" } else { "no-key" },
        if send { "send" } else { "reopen" }
    );
    let (run, saved) = saved_catalog_run(&name, locked).await;
    only_large_4(&run, "local");
    let last = run.resumed().await;
    assert_eq!(last["config"]["models"].as_array().unwrap().len(), 1);
    assert_eq!(last["config"]["models"][0]["source"], "local");
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let before = run.saved();
    if send {
        for method in [
            "turn/start",
            "session/turn/enqueue",
            "session/turn/queue/resume",
        ] {
            let error = super::super::before(&run.core.shared, method, &json!({"sessionId":CHAT}))
                .await
                .unwrap_err();
            assert_eq!(error.code, crate::ErrorCode::InvalidParams);
            assert!(error.message.contains("Unlock the vault"), "{error}");
        }
    } else {
        let restored = super::super::restore(&run.core.shared, CHAT, last.clone(), None).await;
        assert_eq!(restored, last);
    }
    let after = run.saved();
    assert_eq!(
        after, before,
        "the backend identity must stay byte-for-byte"
    );
    assert_eq!(run.state()["writes"], json!([]));
    assert_eq!(after["pending"]["alias"], saved.alias, "{after}");
    assert_eq!(after["remembered"]["alias"], saved.alias, "{after}");
    assert_eq!(alerts(&mut events).await, Vec::<String>::new());
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
            "lockedgone-chat-test-000000000000000",
        ))
        .await
        .unwrap();
    }
    assert!(keys.speech_available());
    std::fs::write(run.home.join("models.json"), models().to_string()).unwrap();
    super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
        .await
        .unwrap();
    assert_eq!(active(&run.resumed().await).0, saved.alias);
    assert_eq!(run.saved(), before);
    assert_eq!(alerts(&mut events).await, Vec::<String>::new());
    run.finish().await;
}

#[tokio::test]
async fn locked_catalog_choice_waits_on_reopen() {
    unavailable_account_choice(true, false).await;
}

#[tokio::test]
async fn locked_catalog_choice_waits_before_send() {
    unavailable_account_choice(true, true).await;
}

#[tokio::test]
async fn no_key_catalog_choice_waits_on_reopen() {
    unavailable_account_choice(false, false).await;
}

#[tokio::test]
async fn no_key_catalog_choice_waits_before_send() {
    unavailable_account_choice(false, true).await;
}

#[tokio::test]
async fn locked_catalog_choice_survives_turn_started_with_local_only_runtime() {
    for old_store_runtime in [true, false] {
        let name = format!("locked-turn-started-{old_store_runtime}");
        let (run, saved) = saved_catalog_run(&name, true).await;
        let previous = run.resumed().await;
        only_large_4(&run, "local");
        let current = run.resumed().await;
        let served = if old_store_runtime { previous } else { current };
        let shared = run.core.shared.clone();
        let _ = shared
            .store
            .load(
                json!({"session":{"id":CHAT,"status":{"type":"idle"}},
                "history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        let _ = shared.store.apply(vw_session::Notification::new(
            "runtime/updated",
            json!({"sessionId":CHAT,"runtime":served}),
        ));
        let before = run.saved();
        let mut events = run.core.subscribe(Some(CHAT.into()));
        super::super::turn_started(&shared, &json!({"sessionId":CHAT})).await;
        assert_eq!(
            run.saved(),
            before,
            "lost saved choice with old_store_runtime={old_store_runtime}"
        );
        assert_eq!(run.saved()["remembered"]["alias"], saved.alias);
        assert_eq!(run.state()["writes"], json!([]));
        assert_eq!(alerts(&mut events).await, Vec::<String>::new());
        run.finish().await;
    }
}

#[tokio::test]
async fn recorded_missing_pin_does_not_replace_an_offered_saved_choice_while_locked() {
    let (run, _) = saved_catalog_run("locked-different-recorded-pin", true).await;
    only_large_4(&run, "local");
    let last = run.resumed().await;
    let local = super::super::identified(
        &run.core.shared,
        CHAT,
        super::super::Choice {
            alias: "mistral-large-4-0".into(),
            name: "mistral-large-4-0".into(),
            display_name: "Mistral Large 4".into(),
            effort: "high".into(),
            provider: String::new(),
            provider_key: String::new(),
        },
    )
    .await;
    run.core
        .shared
        .model_choices
        .records
        .update(CHAT, |record| record.accepted(local))
        .unwrap();
    let before = run.saved();
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let pin =
        super::super::recorded(&json!({"model":"mistral-medium-latest","reasoningEffort":"high"}));
    let restored = super::super::restore(&run.core.shared, CHAT, last.clone(), pin).await;
    assert_eq!(restored, last);
    assert_eq!(run.saved(), before);
    assert_eq!(run.state()["writes"], json!([]));
    assert_eq!(alerts(&mut events).await, Vec::<String>::new());
    run.finish().await;
}

#[tokio::test]
async fn locked_missing_settings_default_warning_refuses_send_without_chat_record() {
    let run = Box::pin(start_with_key_state(
        "locked-warning-only-send",
        &json!({}),
        "mistral-medium-latest",
        None,
        false,
        Some(true),
    ))
    .await;
    only_large_4(&run, "local");
    let mut state = run.state();
    state["warnings"] = json!(["Model 'mistral-medium-latest' is not available"]);
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    let before = run.saved();
    assert_eq!(before, json!({}));
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let error = super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
        .await
        .unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::InvalidParams);
    assert!(error.message.contains("Unlock the vault"), "{error}");
    assert_eq!(run.saved(), before);
    assert_eq!(run.state()["writes"], json!([]));
    assert_eq!(alerts(&mut events).await, Vec::<String>::new());
    run.finish().await;
}

#[tokio::test]
async fn catalog_account_list_makes_missing_chat_model_authoritative() {
    for (send, source) in [
        (false, "catalog"),
        (true, "catalog"),
        (false, "growthbook"),
        (true, "growthbook"),
    ] {
        let name = format!("account-rows-returned-{send}-{source}");
        let (run, saved) = saved_catalog_run(&name, true).await;
        only_large_4(&run, source);
        let last = run.resumed().await;
        assert_eq!(last["config"]["models"][0]["source"], source);
        let before = run.saved();
        let mut events = run.core.subscribe(Some(CHAT.into()));
        if send {
            super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
                .await
                .unwrap();
        } else {
            let _ = super::super::restore(&run.core.shared, CHAT, last, None).await;
        }
        let after = run.saved();
        assert_ne!(
            after, before,
            "authoritative list did not replace unavailable model"
        );
        assert_ne!(after["remembered"]["alias"], saved.alias);
        assert_eq!(after["remembered"]["alias"], "mistral-large-4-0");
        assert_eq!(alerts(&mut events).await.len(), 1);
        run.finish().await;
    }
}

#[tokio::test]
async fn local_only_list_with_usable_key_waits_without_changing_saved_chat_model() {
    for send in [false, true] {
        let name = format!("account-rows-returned-{send}-local");
        let (run, _) = saved_catalog_run(&name, true).await;
        let keys = run.core.shared.config.keys.as_ref().unwrap();
        keys.unlock(
            vw_vault::SecretString::new("refresh notice test vault"),
            None,
        )
        .await
        .unwrap();
        assert!(keys.speech_available());
        only_large_4(&run, "local");
        let last = run.resumed().await;
        assert_eq!(last["config"]["models"][0]["source"], "local");
        let before = run.saved();
        let mut events = run.core.subscribe(Some(CHAT.into()));
        if send {
            let error =
                super::super::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT}))
                    .await
                    .unwrap_err();
            assert_eq!(error.code, crate::ErrorCode::InvalidParams);
            assert_eq!(
                error.message,
                "The saved model for this chat cannot be checked because Vibe Work could not reach Mistral. Check the network or proxy settings, then try Send again."
            );
        } else {
            let restored = super::super::restore(&run.core.shared, CHAT, last.clone(), None).await;
            assert_eq!(restored, last);
        }
        assert_eq!(run.saved(), before);
        assert_eq!(run.state()["writes"], json!([]));
        assert_eq!(alerts(&mut events).await, Vec::<String>::new());
        run.finish().await;
    }
}

#[tokio::test]
async fn locked_catalog_choice_without_pending_record_survives_reopen() {
    for remembered in [true, false] {
        let (run, saved) = saved_catalog_run(&format!("locked-pin-{remembered}"), true).await;
        run.core
            .shared
            .model_choices
            .records
            .update(CHAT, |record| {
                record.pending = None;
                if !remembered {
                    record.remembered = None;
                }
            })
            .unwrap();
        only_large_4(&run, "local");
        let before = run.saved();
        let mut events = run.core.subscribe(Some(CHAT.into()));
        let last = run.resumed().await;
        let restored =
            super::super::restore(&run.core.shared, CHAT, last.clone(), Some(saved)).await;
        assert_eq!(restored, last);
        assert_eq!(run.saved(), before);
        assert_eq!(run.state()["writes"], json!([]));
        assert_eq!(alerts(&mut events).await, Vec::<String>::new());
        run.finish().await;
    }
}
