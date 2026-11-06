//! Config migration regressions for the app-owned runtime; exercise the pure launch editor.
use crate::experiment_pins::with_pins_off;

#[test]
fn obsolete_keys_migrate_without_changing_access_or_personal_comments() {
    for mode in ["ask", "accept-edits", "auto-approve", "smart-approve"] {
        let before = format!(
            "# personal comment\ndefault_agent = '{mode}' # chosen\nsmart_approve_default = false\nexperimental_worktree_copy_on_write = false # keep cloning choice\n[utility_models]\ntitle = 'active' # mine\n"
        );
        let (after, outcome) = with_pins_off(&before);
        assert!(outcome.skipped.is_none());
        let after = after.unwrap();
        let parsed: toml::Table = after.parse().unwrap();
        assert!(!parsed.contains_key("smart_approve_default"));
        assert!(!parsed.contains_key("experimental_worktree_copy_on_write"));
        assert_eq!(parsed["worktree_copy_on_write"].as_bool(), Some(false));
        assert_eq!(parsed["default_agent"].as_str(), Some(mode));
        assert!(after.contains(&format!("default_agent = '{mode}' # chosen")));
        assert!(after.contains("# personal comment"));
        assert!(after.contains("# keep cloning choice"));
        assert!(after.contains("title = 'active' # mine"));
        assert!(with_pins_off(&after).0.is_none(), "migration is idempotent");
    }
}

#[test]
fn explicit_experiments_are_off_and_new_cow_key_wins() {
    let before = "experimental_enable_pr_monitoring = true # personal note\nexperimental_local_runtime_session_api = true\nexperimental_enable_mistral_mcp_connector = true\nexperimental_worktree_copy_on_write = false\nworktree_copy_on_write = true\n";
    let after = with_pins_off(before).0.unwrap();
    let parsed: toml::Table = after.parse().unwrap();
    for key in [
        "experimental_enable_pr_monitoring",
        "experimental_local_runtime_session_api",
        "experimental_enable_mistral_mcp_connector",
    ] {
        assert_eq!(parsed[key].as_bool(), Some(false), "{key}");
    }
    assert_eq!(parsed["worktree_copy_on_write"].as_bool(), Some(true));
    assert!(!parsed.contains_key("experimental_worktree_copy_on_write"));
    assert_eq!(parsed["default_agent"].as_str(), Some("accept-edits"));
    assert!(after.contains("# personal note"));
}

#[test]
fn invalid_config_is_left_for_vibe_to_diagnose() {
    let (after, outcome) = with_pins_off("broken = [");
    assert!(after.is_none());
    assert!(outcome.skipped.is_some());
}

#[test]
fn registry_skills_default_on_without_overwriting_a_persons_choice() {
    let key = "experimental_enable_registry_skills";
    for choice in [None, Some(true), Some(false)] {
        let before = choice.map_or_else(String::new, |on| format!("{key} = {on} # mine\n"));
        let (after, outcome) = with_pins_off(&before);
        assert!(outcome.skipped.is_none());
        let after = after.unwrap();
        let parsed: toml::Table = after.parse().unwrap();
        assert_eq!(parsed[key].as_bool(), Some(choice.unwrap_or(true)));
        if choice.is_some() {
            assert!(after.contains(&before), "the explicit line stays unchanged");
        }
        assert!(with_pins_off(&after).0.is_none());
    }
}

#[test]
fn the_legacy_registry_pin_is_enabled_once_and_personal_choices_then_survive() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let marker = dir.path().join(".vw-registry-skills-default-v1");
    std::fs::write(
        &config,
        "experimental_enable_registry_skills = false # old app pin\ndefault_agent = 'ask'\n",
    )
    .unwrap();
    crate::experiment_pins::ensure(dir.path()).unwrap();
    let text = std::fs::read_to_string(&config).unwrap();
    let mut parsed: toml::Table = text.parse().unwrap();
    assert_eq!(
        parsed["experimental_enable_registry_skills"].as_bool(),
        Some(true)
    );
    assert!(
        marker.is_file(),
        "the one-time migration is recorded outside config.toml"
    );
    assert_eq!(parsed["default_agent"].as_str(), Some("ask"));
    assert!(text.contains("# old app pin"));
    parsed.insert(
        "experimental_enable_registry_skills".into(),
        toml::Value::Boolean(false),
    );
    // Same rewrite as Vibe: comments disappear; the app's separate marker survives.
    std::fs::write(&config, toml::to_string(&parsed).unwrap()).unwrap();
    for _ in 0..2 {
        crate::experiment_pins::ensure(dir.path()).unwrap();
        let after: toml::Table = std::fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(after, parsed, "the person's later false stays false");
        assert!(marker.is_file());
    }
}

#[test]
fn invalid_config_does_not_consume_the_registry_migration() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(&config, "broken = [").unwrap();
    let outcome = crate::experiment_pins::ensure(dir.path()).unwrap();
    assert!(outcome.skipped.is_some());
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "broken = [");
    assert!(!dir.path().join(".vw-registry-skills-default-v1").exists());
}
