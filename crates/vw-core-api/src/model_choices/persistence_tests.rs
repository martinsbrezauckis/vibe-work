//! Persistence regressions for chat choices. Uses real private files, without a Vibe process.
//! Called by cargo test; checks migration, atomic acceptance and the reader/writer size contract.
use super::Choices;
use serde_json::{Value, json};
use std::path::Path;

fn scratch() -> tempfile::TempDir {
    let root =
        std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

fn pick(choices: &Choices, id: &str, alias: &str, label: &str) -> Result<(), crate::CoreError> {
    let effort = "high";
    choices.observe("config/model/write", &json!({"sessionId":id,"modelAlias":alias,"reasoningEffort":effort}),
        &json!({"runtime":{"config":{"activeModel":{"alias":alias,"name":alias,"thinking":effort,"displayName":label}}}}))
}

fn write(dir: &Path, name: &str, value: &Value) {
    std::fs::write(dir.join(name), serde_json::to_vec(value).unwrap()).unwrap();
}

#[test]
fn a_written_choice_larger_than_one_mebibyte_survives_restart() {
    let dir = scratch();
    let choices = Choices::load(Some(dir.path()));
    let label = "M".repeat(1024 * 1024 + 1);
    pick(&choices, "chat", "unavailable-alias", &label).unwrap();
    let restarted = Choices::load(Some(dir.path()));
    let pending = restarted.records.get("chat").pending;
    assert!(
        pending.is_some(),
        "an app-written record was discarded at restart"
    );
    assert_eq!(pending.unwrap().display_name, label);
    assert_eq!(
        restarted.records.get("chat").remembered.unwrap().effort,
        "high"
    );
}

#[test]
fn migration_commits_one_record_and_leaves_the_released_layout_intact() {
    let dir = scratch();
    let choice = json!({"alias":"gone-alias","effort":"provider-future","name":"served-model","display_name":"Saved model"});
    for (name, id) in [
        ("pending-models.json", "pending"),
        ("chat-models.json", "accepted"),
        ("model-fallbacks.json", "fallback"),
    ] {
        write(dir.path(), name, &json!({id:choice}));
    }
    let choices = Choices::load(Some(dir.path()));
    assert_eq!(
        choices.records.get("pending").pending.unwrap().effort,
        "provider-future"
    );
    assert_eq!(
        choices.records.get("accepted").remembered.unwrap().name,
        "served-model"
    );
    assert_eq!(
        choices.records.get("fallback").fallback.unwrap().alias,
        "gone-alias"
    );
    assert!(
        dir.path().join("model-choices.json").exists(),
        "migration did not commit a unified record"
    );
    for (name, id) in [
        ("pending-models.json", "pending"),
        ("chat-models.json", "accepted"),
        ("model-fallbacks.json", "fallback"),
    ] {
        let value: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join(name)).unwrap()).unwrap();
        assert_eq!(value, json!({id:choice}), "downgrade snapshot changed");
    }
}

#[test]
fn a_failed_acceptance_does_not_publish_half_a_choice() {
    let dir = scratch();
    let old = json!({"alias":"old","effort":"off"});
    write(dir.path(), "pending-models.json", &json!({"chat":old}));
    write(dir.path(), "chat-models.json", &json!({"chat":old}));
    let choices = Choices::load(Some(dir.path()));
    // The old implementation commits pending, then fails when replacing remembered.
    std::fs::remove_file(dir.path().join("chat-models.json")).unwrap();
    std::fs::create_dir(dir.path().join("chat-models.json")).unwrap();
    // The unified store must fail before committing any portion of the change.
    let unified = dir.path().join("model-choices.json");
    if unified.exists() {
        std::fs::remove_file(&unified).unwrap();
    }
    std::fs::create_dir(&unified).unwrap();
    assert!(pick(&choices, "chat", "new", "New").is_err());
    assert_eq!(
        choices.records.get("chat").pending.unwrap().alias,
        "old",
        "failed write published a partial accepted choice"
    );
}

#[test]
fn a_failed_handoff_survives_the_next_save_without_replacing_a_newer_pick() {
    for newer_pick in [false, true] {
        let dir = scratch();
        let choices = Choices::load(Some(dir.path()));
        pick(&choices, "source", "chosen", "Chosen").unwrap();
        let file = dir.path().join("model-choices.json");
        let before = std::fs::read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        super::handed_off(&choices, "source", "replacement");
        assert_eq!(
            choices.records.get("replacement").pending.unwrap().alias,
            "chosen"
        );
        std::fs::remove_dir(&file).unwrap();
        std::fs::write(&file, before).unwrap();
        if newer_pick {
            let another_window = Choices::load(Some(dir.path()));
            pick(&another_window, "replacement", "newer", "Newer").unwrap();
        }
        // An otherwise unchanged pick must also flush the unsaved handoff.
        pick(&choices, "source", "chosen", "Chosen").unwrap();
        let restarted = Choices::load(Some(dir.path()));
        let replacement = restarted.records.get("replacement").pending;
        assert!(
            replacement.is_some(),
            "a later save lost the handoff held in memory"
        );
        assert_eq!(
            replacement.unwrap().alias,
            if newer_pick { "newer" } else { "chosen" }
        );
        assert_eq!(
            restarted.records.get("source").pending.unwrap().alias,
            "chosen"
        );
    }
}
