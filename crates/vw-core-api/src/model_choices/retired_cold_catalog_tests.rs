//! Cold-resume catalog alias regression and missing identity controls, using the retired-model stand-in.
//! The parent test module owns the fake Vibe and durable chat record fixture.
#![allow(clippy::unwrap_used)]

use super::super as model_choices;
use super::*;

fn reset_cold(run: &Run) {
    let mut state = run.state();
    state["active"] = json!("mistral-large-4-0");
    state["writes"] = json!([]);
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    std::fs::remove_file(run.home.join("catalog-ready")).unwrap();
}

async fn remembered_catalog_replays(run: &Run, events: &mut crate::CoreSubscription) {
    run.core
        .shared
        .model_choices
        .records
        .update(CHAT, |record| record.pending = None)
        .unwrap();
    reset_cold(run);
    let pin =
        model_choices::recorded(&json!({"model":"mistral-medium-latest","reasoningEffort":"high"}));
    let remembered = model_choices::restore(&run.core.shared, CHAT, run.resumed().await, pin).await;
    assert_eq!(
        active(&remembered),
        ("mistral-medium-latest".into(), "high".into())
    );
    assert_eq!(alerts(events).await, Vec::<String>::new());
}

async fn cold_catalog_fallback_controls(
    run: &Run,
    saved: model_choices::Choice,
    events: &mut crate::CoreSubscription,
) {
    // A changed backend never inherits the saved pick.
    run.core
        .shared
        .model_choices
        .records
        .update(CHAT, |record| record.accepted(saved.clone()))
        .unwrap();
    let mut changed = models();
    changed.as_array_mut().unwrap()[1]["provider"] = json!("llamacpp");
    std::fs::write(run.home.join("models.json"), changed.to_string()).unwrap();
    reset_cold(run);
    let changed = model_choices::restore(&run.core.shared, CHAT, run.resumed().await, None).await;
    assert_eq!(active(&changed), large_4("high"));
    assert_eq!(
        alerts(events).await.len(),
        1,
        "backend change needs one notice"
    );
    let reopened = model_choices::restore(&run.core.shared, CHAT, run.resumed().await, None).await;
    assert_eq!(active(&reopened), large_4("high"));
    assert_eq!(
        alerts(events).await,
        Vec::<String>::new(),
        "notice repeated"
    );

    // A removed row also gets one notice across two opens.
    run.core
        .shared
        .model_choices
        .records
        .update(CHAT, |record| record.accepted(saved.clone()))
        .unwrap();
    std::fs::write(
        run.home.join("models.json"),
        json!([models()[0]]).to_string(),
    )
    .unwrap();
    reset_cold(run);
    let missing = model_choices::restore(&run.core.shared, CHAT, run.resumed().await, None).await;
    assert_eq!(active(&missing), large_4("high"));
    assert_eq!(
        alerts(events).await.len(),
        1,
        "removed model needs one notice"
    );
    let reopened = model_choices::restore(&run.core.shared, CHAT, run.resumed().await, None).await;
    assert_eq!(active(&reopened), large_4("high"));
    assert_eq!(
        alerts(events).await,
        Vec::<String>::new(),
        "removed model notice repeated"
    );

    // Missing resolved fields cannot authorize replay from only a saved digest.
    run.core
        .shared
        .model_choices
        .records
        .update(CHAT, |record| record.accepted(saved))
        .unwrap();
    std::fs::write(run.home.join("models.json"), models().to_string()).unwrap();
    std::fs::write(run.home.join("no-fields"), "").unwrap();
    reset_cold(run);
    let missing_fields =
        model_choices::restore(&run.core.shared, CHAT, run.resumed().await, None).await;
    assert_eq!(active(&missing_fields), large_4("high"));
    assert_eq!(
        alerts(events).await.len(),
        1,
        "missing fields need a notice"
    );
}

/// The installed Mac resumed with only configured aliases in runtime/read, although the
/// catalog alias selected before quit remained in config/fields/read's resolved models.
#[tokio::test]
async fn a_catalog_pick_survives_cold_resume_with_configured_aliases_only() {
    let run = Box::pin(start(
        "catalog-cold-resume",
        &json!({}),
        "mistral-medium-latest",
    ))
    .await;
    let mut state = run.state();
    state["active"] = json!("mistral-large-4-0");
    state["default"] = json!("mistral-large-4-0");
    state["warnings"] = json!([]);
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    std::fs::write(run.home.join("cold-models.json"), json!([
        {"alias":"mistral-large-4-0","name":"mistral-large-4-0","provider":"mistral",
         "displayName":"Mistral Large 4","thinking":"high","thinkingLevels":["low","medium","high"]},
        {"alias":"mistral-medium-3.5","name":"mistral-vibe-cli-latest","provider":"mistral",
         "displayName":"Mistral Medium 3.5","thinking":"high","thinkingLevels":["off","high"]}
    ]).to_string()).unwrap();
    let saved = model_choices::identified(
        &run.core.shared,
        CHAT,
        model_choices::Choice {
            alias: "mistral-medium-latest".into(),
            name: "mistral-medium-latest".into(),
            display_name: "Mistral Medium 3.5".into(),
            effort: "high".into(),
            provider: String::new(),
            provider_key: String::new(),
        },
    )
    .await;
    assert!(
        !saved.provider_key.is_empty(),
        "catalog model needs a proven backend"
    );
    std::fs::remove_file(run.home.join("catalog-ready")).unwrap();
    run.core
        .shared
        .model_choices
        .records
        .update(CHAT, |record| record.accepted(saved.clone()))
        .unwrap();
    let last = run.resumed().await;
    assert_eq!(active(&last), large_4("high"));
    assert_eq!(last["config"]["models"].as_array().unwrap().len(), 2);
    assert!(
        last["config"]["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["alias"] != "mistral-medium-latest")
    );
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let restored = model_choices::restore(&run.core.shared, CHAT, last, None).await;
    assert_eq!(
        active(&restored),
        ("mistral-medium-latest".into(), "high".into()),
        "{restored}"
    );
    assert_eq!(alerts(&mut events).await, Vec::<String>::new());
    assert_eq!(run.saved()["pending"]["alias"], "mistral-medium-latest");
    remembered_catalog_replays(&run, &mut events).await;
    cold_catalog_fallback_controls(&run, saved, &mut events).await;
    run.finish().await;
}

#[tokio::test]
async fn a_new_pick_without_resolved_fields_is_not_saved_without_provenance() {
    let run = Box::pin(start("no-fields", &json!({}), "large-4")).await;
    std::fs::write(run.home.join("no-fields"), "").unwrap();
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let reply = model_choices::pick(
        &run.core.shared,
        &json!({"sessionId":CHAT,"sessionOnly":true,"modelAlias":"mistral-medium-latest",
            "reasoningEffort":"high"}),
        None,
    )
    .await
    .unwrap();
    assert_eq!(reply.result["modelChoiceFallback"], true);
    assert!(
        run.saved()["remembered"].is_null(),
        "unproven pick was saved"
    );
    assert_eq!(alerts(&mut events).await.len(), 1);
    run.finish().await;
}
