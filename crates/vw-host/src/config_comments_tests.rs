//! Re-add app-owned explanations after a config serializer removed them; personal lines and values stay exact.
use crate::{commit_signature, experiment_pins, title_model, trust_store};

#[test]
fn next_start_restores_config_explanations_without_rewriting_personal_lines() {
    let root = std::path::PathBuf::from(
        std::env::var_os("VW_TEST_ROOT").unwrap_or_else(|| std::env::temp_dir().into_os_string()),
    )
    .join(format!("config-comments-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("config.toml");
    // Vibe's rewrite has retained the values, dropped our notes, and the person added a line of their own.
    let original = "# my own explanation\ninclude_commit_signature = false # mine\nenable_system_trust_store = true\nexperimental_enable_pr_monitoring = false\nexperimental_enable_registry_skills = false\nsmart_approve_default = false\n\n[utility_models]\ntitle = 'mistral-medium-3.5' # keep this spelling\n";
    std::fs::write(&path, original).unwrap();
    for _ in 0..2 {
        commit_signature::ensure(&root).unwrap();
        trust_store::ensure(&root).unwrap();
        experiment_pins::ensure(&root).unwrap();
        title_model::ensure(&root).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        for phrase in [
            "# Vibe Work: the agent's commits",
            "# Vibe Work: Vibe checks HTTPS",
            "# Vibe Work: Mistral's experiments",
            "# Vibe Work: session titles",
        ] {
            assert_eq!(
                after.matches(phrase).count(),
                1,
                "missing or repeated explanation: {phrase}"
            );
        }
        let own = after
            .lines()
            .filter(|line| {
                !line.starts_with("# Vibe Work:")
                    && !line.starts_with("# (Your own commits:")
                    && !line.starts_with("# Vibe's Python packages,")
                    && !line.starts_with("# Write title =")
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(own, original);
        assert_eq!(
            after.parse::<toml::Table>().unwrap(),
            original.parse::<toml::Table>().unwrap()
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
