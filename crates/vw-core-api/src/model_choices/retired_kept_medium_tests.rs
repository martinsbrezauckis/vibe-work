//! Medium 3.5 is a built-in row of Vibe 3.1.4 that Mistral's own list leaves out, and it is no longer kept alive:
//! a chat that saved it gets the ordinary "unavailable" notice and the Settings default, and the row that 0.1.16 and
//! 0.1.17 copied into the app's Vibe config is taken out again. The fake Vibe reads app-owned config.toml as the
//! higher-priority layer (a `[[models]]` row there is offered).
use super::*;

/// The row and comment 0.1.16 and 0.1.17 appended to config.toml, byte for byte.
const KEPT_ROW: &str = r#"

# Vibe Work: retain a model this person picked before Vibe's catalog replaced its built-in list.
[[models]]
name = "mistral-vibe-cli-latest"
provider = "mistral"
alias = "mistral-medium-3.5"
display_name = "Mistral Medium 3.5"
temperature = 1.0
input_price = 1.5
output_price = 7.5
cached_input_price = 0.15
thinking = "high"
thinking_levels = ["off", "high"]
supports_images = true
max_context_length = 262144
"#;
const UNTOUCHED: &str = "active_model = \"mistral-large-4-0\"\n";

/// A saved built-in Medium 3.5 choice with the backend proof a chat that really used it carries.
fn saved_medium() -> Value {
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
        .unwrap()
        .1;
    json!({"pending":selected,"remembered":selected})
}

/// The chat reopens: Vibe resumed it on the Settings default (Medium from the catalog, `mistral-medium-latest`).
async fn reopen_gets_the_notice(run: &Run) {
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let resumed = run.resumed().await;
    assert_eq!(active(&resumed).0, "mistral-medium-latest");
    let restored = super::super::restore(&run.core.shared, CHAT, resumed, None).await;
    assert_eq!(active(&restored).0, "mistral-medium-latest", "{restored}");
    let seen = alerts(&mut events).await;
    assert_eq!(seen.len(), 1, "one notice, nothing silent: {seen:?}");
    assert!(
        seen[0].contains("Mistral Medium 3.5") && seen[0].contains("unavailable"),
        "{}",
        seen[0]
    );
    assert_eq!(run.saved()["remembered"]["alias"], "mistral-medium-latest");
}

#[tokio::test]
async fn a_chat_that_saved_the_built_in_medium_gets_the_unavailable_notice() {
    let run = Box::pin(start_with_config(
        "builtin-medium-gone",
        &saved_medium(),
        "mistral-medium-3.5",
        Some(UNTOUCHED),
    ))
    .await;
    let config = std::fs::read_to_string(run.home.join("config.toml")).unwrap();
    assert_eq!(config, UNTOUCHED, "no row is kept for a saved Medium 3.5");
    reopen_gets_the_notice(&run).await;
    run.finish().await;
}

#[tokio::test]
async fn the_row_an_older_version_kept_is_removed_and_the_chat_gets_the_notice() {
    let run = Box::pin(start_with_marker(
        "builtin-medium-upgrade",
        &saved_medium(),
        "mistral-medium-3.5",
        Some(&format!("{UNTOUCHED}{KEPT_ROW}")),
        true,
    ))
    .await;
    let config = std::fs::read_to_string(run.home.join("config.toml")).unwrap();
    assert_eq!(config, UNTOUCHED, "the kept row and its comment are gone");
    assert!(
        !run.sessions.join("built-in-medium-choice-v1").exists(),
        "the marker is gone"
    );
    reopen_gets_the_notice(&run).await;
    run.finish().await;
}
