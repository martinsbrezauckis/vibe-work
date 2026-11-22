//! A real 3.1.4 restart can replace the built-in Medium row with catalog rows before resume.
//! The fake Vibe reads app-owned config.toml as the higher-priority local layer.
use super::*;

#[tokio::test]
async fn saved_builtin_medium_is_restored_after_catalog_replaces_defaults() {
    let mut selected = super::super::Choice {
        alias: "mistral-medium-3.5".into(),
        name: "mistral-vibe-cli-latest".into(),
        display_name: "Mistral Medium 3.5".into(),
        effort: "high".into(),
        provider: "mistral".into(),
        provider_key: String::new(),
    };
    let fields = json!({"fields":[
        {"name":"models","value":{"mistral-medium-3.5":{
            "alias":"mistral-medium-3.5","name":"mistral-vibe-cli-latest","provider":"mistral"}}},
        {"name":"providers","value":[{"name":"mistral","api_base":"http://127.0.0.1:19462/v1",
            "api_style":"openai","backend":"generic"}]}
    ]});
    selected.provider_key = super::super::provenance(&fields, &selected, None)
        .expect("the captured model and route identify the backend")
        .1;
    let saved = json!({"pending":selected,"remembered":selected});
    let run = Box::pin(start_with_config(
        "builtin-medium-catalog-restart",
        &saved,
        "mistral-medium-3.5",
        Some("active_model = \"mistral-large-4-0\"\n"),
    ))
    .await;
    let config = std::fs::read_to_string(run.home.join("config.toml")).unwrap();
    assert!(
        config.contains("alias = \"mistral-medium-3.5\""),
        "the saved built-in row must be available before Vibe starts: {config}"
    );
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let resumed = run.resumed().await;
    assert_eq!(active(&resumed).0, "mistral-medium-latest");
    let restored = super::super::restore(&run.core.shared, CHAT, resumed, None).await;
    assert_eq!(active(&restored).0, "mistral-medium-3.5");
    assert_eq!(run.saved()["pending"]["alias"], "mistral-medium-3.5");
    assert_eq!(run.saved()["remembered"]["alias"], "mistral-medium-3.5");
    assert_eq!(alerts(&mut events).await, Vec::<String>::new());
    run.finish().await;
}
