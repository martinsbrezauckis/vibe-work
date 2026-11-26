//! Persistence regressions for chat choices. Uses real private files, without a Vibe process.
//! Called by cargo test; checks migration, atomic acceptance and the reader/writer size contract.
use super::{Choices, actual, provenance};
use serde_json::{Value, json};
use std::path::Path;

fn scratch() -> tempfile::TempDir {
    let root =
        std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

#[test]
fn the_resolved_provider_route_changes_identity_without_a_wire_model_change() {
    // activeModel is the shape captured from the pinned 3.1.4 server: provider is absent.
    let runtime = json!({"runtime":{"config":{"activeModel":{
        "alias":"same","name":"same-name","thinking":"route-level","supportsImages":false,
        "displayName":"same","source":"local","thinkingLevels":["off","route-level"]
    }}}});
    let choice = actual(&runtime).unwrap();
    assert_eq!(choice.provider, "");
    let fields = |provider: &str, base: &str| {
        json!({"fields":[
        {"name":"models","kind":"complex","description":"","path":"/models",
         "value":{"same":{"alias":"same","name":"same-name","provider":provider}}},
        {"name":"providers","kind":"complex","description":"","path":"/providers",
         "value":[{"name":provider,"api_base":base,"api_style":"openai",
                   "backend":"generic"}]}
    ],"targets":[]})
    };
    let first = provenance(
        &fields("llamacpp", "http://127.0.0.1:19462/v1"),
        &choice,
        None,
    )
    .unwrap();
    let changed_provider = provenance(
        &fields("mistral", "http://127.0.0.1:19462/v1"),
        &choice,
        None,
    )
    .unwrap();
    let changed_endpoint = provenance(
        &fields("llamacpp", "http://127.0.0.1:19463/v1"),
        &choice,
        None,
    )
    .unwrap();
    assert_eq!(first.0, "llamacpp");
    assert_ne!(first.1, changed_provider.1);
    assert_ne!(first.1, changed_endpoint.1);
    assert!(
        provenance(
            &fields("llamacpp", "http://127.0.0.1:19462/v1"),
            &super::Choice {
                name: "different-name".into(),
                ..choice.clone()
            },
            None
        )
        .is_none()
    );
    assert!(provenance(&json!({"fields":[]}), &choice, None).is_none());
}

#[test]
fn app_proxy_port_is_stable_but_a_custom_mistral_route_is_distinct() {
    let choice = actual(&json!({"runtime":{"config":{"activeModel":{
        "alias":"route-choice","name":"route-choice","thinking":"route-level"
    }}}}))
    .unwrap();
    let fields = |base: &str, backend: &str| {
        json!({"fields":[
            {"name":"models","value":{"route-choice":{"alias":"route-choice","name":"route-choice","provider":"mistral"}}},
            {"name":"providers","value":[{"name":"mistral","api_base":base,
                "api_style":"openai","backend":backend,"api_key_env_var":"MISTRAL_API_KEY"}]}
        ]})
    };
    let first_route = "http://127.0.0.1:19462/v1";
    let second_route = "http://127.0.0.1:19463/v1";
    let first = provenance(&fields(first_route, "mistral"), &choice, Some(first_route)).unwrap();
    let second = provenance(
        &fields(second_route, "mistral"),
        &choice,
        Some(second_route),
    )
    .unwrap();
    let custom = provenance(
        &fields(second_route, "generic"),
        &choice,
        Some(second_route),
    )
    .unwrap();
    let different_route =
        provenance(&fields(second_route, "mistral"), &choice, Some(first_route)).unwrap();
    assert_eq!(first, second);
    assert_ne!(first, custom);
    assert_ne!(first, different_route);
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

#[test]
fn detached_picks_overlay_both_fields_without_mutating_pins_or_other_chats() {
    let dir = scratch();
    write(
        dir.path(),
        "model-choices.json",
        &json!({"version":1,"chats":{
            "first":{"pending":{"alias":"chosen","effort":"low","name":"chosen-name","provider":"llamacpp"}},
            "second":{"pending":{"alias":"other","effort":"max"}}
        }}),
    );
    let file = dir.path().join("model-choices.json");
    let before = std::fs::read(&file).unwrap();
    let choices = Choices::load(Some(dir.path()));
    let original = json!({"id":"first","model":"old","reasoningEffort":"high","title":"kept"});
    assert_eq!(
        choices.detached_session(&original),
        (
            json!({"id":"first","model":"chosen","reasoningEffort":"low","title":"kept"}),
            "chosen-name".into(),
            "llamacpp".into()
        )
    );
    assert_eq!(original["model"], "old");
    assert_eq!(original["reasoningEffort"], "high");
    assert_eq!(
        choices.detached_session(&json!({"id":"second"})).0,
        json!({"id":"second","model":"other","reasoningEffort":"max"})
    );
    assert_eq!(std::fs::read(file).unwrap(), before);
}

#[test]
fn detached_missing_invalid_and_remembered_only_picks_keep_existing_fields() {
    let dir = scratch();
    write(
        dir.path(),
        "model-choices.json",
        &json!({"version":1,"chats":{
            "remembered":{"remembered":{"alias":"older","effort":"low"}},
            "empty_alias":{"pending":{"alias":"","effort":"low"}},
            "empty_effort":{"pending":{"alias":"chosen","effort":""}},
            "invalid_effort":{"pending":{"alias":"chosen","effort":"bad level"}},
            "damaged":{"pending":{"alias":"chosen","effort":null}}
        }}),
    );
    let choices = Choices::load(Some(dir.path()));
    for id in [
        "missing",
        "remembered",
        "empty_alias",
        "empty_effort",
        "invalid_effort",
        "damaged",
    ] {
        let session = json!({"id":id,"model":"pinned","reasoningEffort":"high"});
        assert_eq!(choices.detached_session(&session).0, session, "{id}");
    }
    for session in [Value::Null, json!({}), json!({"id":null}), json!({"id":7})] {
        assert_eq!(choices.detached_session(&session).0, session);
    }
}

#[test]
fn detached_backend_bound_remembered_choices_show_each_chats_model_and_effort() {
    let dir = scratch();
    write(
        dir.path(),
        "model-choices.json",
        &json!({"version":1,"chats":{
            "first":{"remembered":{"alias":"saved","effort":"low","name":"saved-wire",
                "provider":"mistral","provider_key":"route-one","display_name":"Saved model"}},
            "second":{"remembered":{"alias":"other","effort":"off","name":"other-wire",
                "provider":"mistral","provider_key":"route-two","display_name":"Other model"}}
        }}),
    );
    let file = dir.path().join("model-choices.json");
    let before = std::fs::read(&file).unwrap();
    let choices = Choices::load(Some(dir.path()));
    let first = json!({"id":"first","model":"runtime-fallback","reasoningEffort":"high"});
    let second = json!({"id":"second","model":"runtime-fallback","reasoningEffort":"high"});
    assert_eq!(
        choices.detached_session(&first),
        (
            json!({"id":"first","model":"saved","reasoningEffort":"low"}),
            "saved-wire".into(),
            "mistral".into()
        )
    );
    assert_eq!(
        choices.detached_session(&second),
        (
            json!({"id":"second","model":"other","reasoningEffort":"off"}),
            "other-wire".into(),
            "mistral".into()
        )
    );
    assert_eq!(first["model"], "runtime-fallback");
    assert_eq!(second["model"], "runtime-fallback");
    assert_eq!(std::fs::read(file).unwrap(), before);
}

#[test]
fn detached_fallback_keeps_the_same_precedence_as_attached_replay() {
    let dir = scratch();
    write(
        dir.path(),
        "model-choices.json",
        &json!({"version":1,"chats":{
            "chat":{
                "pending":{"alias":"chosen","effort":"low"},
                "fallback":{"alias":"replacement","effort":"off"},
                "remembered":{"alias":"older","effort":"high"}
            }
        }}),
    );
    let choices = Choices::load(Some(dir.path()));
    assert_eq!(
        choices
            .detached_session(&json!({"id":"chat","model":"old","reasoningEffort":"high"}))
            .0,
        json!({"id":"chat","model":"replacement","reasoningEffort":"off"})
    );
}
