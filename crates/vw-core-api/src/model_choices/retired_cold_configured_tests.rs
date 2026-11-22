//! A saved configured alias must wait for a cold session's model list before its first Send.
//! The parent test module supplies the fake Vibe, durable choice record and event reader.
#![allow(clippy::unwrap_used)]

use super::super as model_choices;
use super::*;

trait SendOutcome {
    fn refused(self) -> bool;
}

impl SendOutcome for () {
    fn refused(self) -> bool {
        false
    }
}

impl SendOutcome for Result<(), crate::error::CoreError> {
    fn refused(self) -> bool {
        self.is_err()
    }
}

#[tokio::test]
async fn a_configured_pick_survives_cold_catalog_rows_before_first_send() {
    let run = Box::pin(start(
        "configured-cold-resume",
        &json!({}),
        "mistral-large-4-0",
    ))
    .await;
    let configured = json!([
        {"alias":"mistral-large-4-0","name":"mistral-large-4-0","provider":"mistral",
         "displayName":"Mistral Large 4","thinking":"high","thinkingLevels":["low","medium","high"]},
        {"alias":"mistral-medium-3.5","name":"mistral-vibe-cli-latest","provider":"mistral",
         "displayName":"Mistral Medium 3.5","thinking":"high","thinkingLevels":["off","high"]}
    ]);
    std::fs::write(run.home.join("models.json"), configured.to_string()).unwrap();
    let saved = model_choices::identified(
        &run.core.shared,
        CHAT,
        model_choices::Choice {
            alias: "mistral-medium-3.5".into(),
            name: "mistral-vibe-cli-latest".into(),
            display_name: "Mistral Medium 3.5".into(),
            effort: "high".into(),
            provider: String::new(),
            provider_key: String::new(),
        },
    )
    .await;
    assert_ne!(saved.provider_key, "", "the pick needs a proven backend");
    run.core
        .shared
        .model_choices
        .records
        .update(CHAT, |record| record.accepted(saved.clone()))
        .unwrap();
    let mut state = run.state();
    state["active"] = json!("mistral-large-4-0");
    state["default"] = json!("mistral-large-4-0");
    state["warnings"] = json!([]);
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    // The selected chat is detached after restart. Its first runtime has catalog
    // rows only; the configured rows arrive when session/ready/wait completes.
    std::fs::write(run.home.join("cold-models.json"), models().to_string()).unwrap();
    std::fs::write(run.home.join("wait-to-ready"), "").unwrap();
    std::fs::remove_file(run.home.join("catalog-ready")).unwrap();
    let cold_reply = run
        .core
        .shared
        .gate
        .call("runtime/read", json!({"sessionId":CHAT}), None)
        .await
        .unwrap();
    assert_eq!(cold_reply.result["ready"], false);
    let cold = run.resumed().await;
    assert_eq!(active(&cold), large_4("high"));
    assert!(!model_choices::restore::offered(&cold, &saved));
    assert_eq!(
        cold["config"]["models"][1]["alias"],
        "mistral-medium-latest"
    );
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let restored = model_choices::restore(&run.core.shared, CHAT, cold, None).await;
    let ready_reply = run
        .core
        .shared
        .gate
        .call("runtime/read", json!({"sessionId":CHAT}), None)
        .await
        .unwrap();
    assert_eq!(ready_reply.result["ready"], true);
    assert_eq!(
        active(&restored),
        ("mistral-medium-3.5".into(), "high".into())
    );
    assert_eq!(run.saved()["pending"]["alias"], saved.alias);
    assert_eq!(run.saved()["remembered"]["alias"], saved.alias);
    assert_eq!(alerts(&mut events).await.len(), 0);
    let _ = model_choices::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT})).await;
    assert_eq!(
        active(&run.resumed().await),
        ("mistral-medium-3.5".into(), "high".into())
    );
    assert_eq!(run.saved()["remembered"]["alias"], saved.alias);
    assert_eq!(alerts(&mut events).await.len(), 0);
    run.finish().await;
}

#[tokio::test]
async fn a_failed_cold_readiness_wait_keeps_the_saved_pick() {
    let run = Box::pin(start(
        "configured-wait-failed",
        &json!({}),
        "mistral-large-4-0",
    ))
    .await;
    std::fs::write(
        run.home.join("models.json"),
        json!([
            models()[0],
            {"alias":"mistral-medium-3.5","name":"mistral-vibe-cli-latest","provider":"mistral",
             "displayName":"Mistral Medium 3.5","thinking":"high","thinkingLevels":["off","high"]}
        ])
        .to_string(),
    )
    .unwrap();
    let saved = model_choices::identified(
        &run.core.shared,
        CHAT,
        model_choices::Choice {
            alias: "mistral-medium-3.5".into(),
            name: "mistral-vibe-cli-latest".into(),
            display_name: "Mistral Medium 3.5".into(),
            effort: "high".into(),
            provider: String::new(),
            provider_key: String::new(),
        },
    )
    .await;
    assert_ne!(saved.provider_key, "");
    run.core
        .shared
        .model_choices
        .records
        .update(CHAT, |record| record.accepted(saved.clone()))
        .unwrap();
    let mut state = run.state();
    state["active"] = json!("mistral-large-4-0");
    state["default"] = json!("mistral-large-4-0");
    state["warnings"] = json!([]);
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    std::fs::write(run.home.join("cold-models.json"), models().to_string()).unwrap();
    std::fs::write(run.home.join("wait-to-ready"), "").unwrap();
    std::fs::write(run.home.join("wait-fails"), "").unwrap();
    std::fs::remove_file(run.home.join("catalog-ready")).unwrap();
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let cold = run.resumed().await;
    let deferred = model_choices::restore(&run.core.shared, CHAT, cold, None).await;
    assert_eq!(active(&deferred), large_4("high"));
    assert_eq!(run.saved()["pending"]["alias"], saved.alias);
    assert_eq!(run.saved()["remembered"]["alias"], saved.alias);
    assert_eq!(alerts(&mut events).await.len(), 0);
    let send =
        model_choices::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT})).await;
    assert!(
        SendOutcome::refused(send),
        "an unready runtime must refuse the first Send"
    );
    assert_eq!(run.saved()["pending"]["alias"], saved.alias);
    assert_eq!(alerts(&mut events).await.len(), 0);
    run.finish().await;
}

#[tokio::test]
async fn a_failed_cold_model_write_keeps_the_saved_pick() {
    let run = Box::pin(start(
        "configured-write-failed",
        &json!({}),
        "mistral-large-4-0",
    ))
    .await;
    std::fs::write(
        run.home.join("models.json"),
        json!([
            models()[0],
            {"alias":"mistral-medium-3.5","name":"mistral-vibe-cli-latest","provider":"mistral",
             "displayName":"Mistral Medium 3.5","thinking":"high","thinkingLevels":["off","high"]}
        ])
        .to_string(),
    )
    .unwrap();
    let saved = model_choices::identified(
        &run.core.shared,
        CHAT,
        model_choices::Choice {
            alias: "mistral-medium-3.5".into(),
            name: "mistral-vibe-cli-latest".into(),
            display_name: "Mistral Medium 3.5".into(),
            effort: "high".into(),
            provider: String::new(),
            provider_key: String::new(),
        },
    )
    .await;
    assert_ne!(saved.provider_key, "");
    run.core
        .shared
        .model_choices
        .records
        .update(CHAT, |record| record.accepted(saved.clone()))
        .unwrap();
    let mut state = run.state();
    state["active"] = json!("mistral-large-4-0");
    state["default"] = json!("mistral-large-4-0");
    state["warnings"] = json!([]);
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    std::fs::write(run.home.join("cold-models.json"), models().to_string()).unwrap();
    std::fs::write(run.home.join("wait-to-ready"), "").unwrap();
    std::fs::write(run.home.join("write-fails"), "").unwrap();
    std::fs::remove_file(run.home.join("catalog-ready")).unwrap();
    let cold = run.resumed().await;
    let restored = model_choices::restore(&run.core.shared, CHAT, cold, None).await;
    assert_eq!(active(&restored), large_4("high"));
    assert_eq!(run.saved()["pending"]["alias"], saved.alias);
    assert_eq!(run.saved()["remembered"]["alias"], saved.alias);
    let send =
        model_choices::before(&run.core.shared, "turn/start", &json!({"sessionId":CHAT})).await;
    assert!(
        SendOutcome::refused(send),
        "a failed replay must refuse Send"
    );
    assert_eq!(run.saved()["pending"]["alias"], saved.alias);
    run.finish().await;
}
