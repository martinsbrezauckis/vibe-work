//! Pinned real Vibe in disposable homes, with its provider routed only to a loopback scripted model.
//! Shared by protocol fixtures; the supervisor still owns the real process, sessions, tools and persistence.

use std::path::{Path, PathBuf};

use vw_supervisor::SupervisorConfig;

use super::scripted_model::{Script, ScriptedModel};

pub const PIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../tools/public/codegen/vibe.pin"
));
pub const PLACEHOLDER_KEY: &str = "vw-scripted-model-placeholder-not-a-key";

/// The source root, without a Windows verbatim prefix (Git does not accept that prefix).
pub fn repo_root() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// The pinned versioned environment, with every explicit override checked before use.
pub fn server_bin() -> PathBuf {
    vw_public_tests::runtime::selected()
        .unwrap_or_else(|error| panic!("{error}"))
        .server
}

/// The pinned Vibe's own Python: the interpreter of [`server_bin`]'s environment. On Windows a uv tool launcher
/// has none beside it and names it inside the file instead ([`vw_supervisor::server_python`]).
pub fn python() -> PathBuf {
    let server = server_bin();
    vw_supervisor::server_python(&server)
        .unwrap_or_else(|| panic!("no Python found for the pinned Vibe {}", server.display()))
}

/// A local text model for reads/startup probes; tests that need a tool supply their own Script.
pub fn text_script() -> Script {
    serde_json::from_value(serde_json::json!({"default":{"text":"Scripted reply"}})).unwrap()
}

/// Real write_file effects for edit:<path> requests, followed by a final answer.
pub fn edit_script() -> Script {
    serde_json::from_value(serde_json::json!({"rules":[
        {"side":true,"reply":{"text":"Scripted title"}},
        {"step":0,"reply":{"editTokens":true}},
        {"reply":{"text":"Scripted reply"}}
    ]}))
    .unwrap()
}

/// One harmless real write per turn, followed by text. Ask mode is chosen explicitly by its caller.
pub fn approval_script() -> Script {
    serde_json::from_value(serde_json::json!({"rules":[
        {"side":true,"reply":{"text":"Scripted title"}},
        {"user":"^go ([A-Za-z0-9-]+)$","step":0,"reply":{"tools":[{"name":"write_file","args":{"path":"${1}.txt","content":"scripted tool\n"}}]}},
        {"step":0,"reply":{"tools":[{"name":"write_file","args":{"path":"approval.txt","content":"scripted tool\n"}}]}},
        {"reply":{"text":"Scripted reply"}}
    ]})).unwrap()
}

/// The same model aliases as a normal test home, with telemetry/connectors disabled and no remote provider.
pub fn vibe_config(model: &ScriptedModel) -> String {
    format!(
        r#"enable_telemetry = false
smart_approve_available = true
enable_connectors = {}
experimental_enable_registry_skills = false
include_commit_signature = false
active_model = "large-4"

[[models]]
name = "mistral-vibe-cli-latest"
provider = "mistral"
alias = "mistral-medium-3.5"
thinking = "high"
supports_images = true

[[models]]
name = "mistral-large-4"
provider = "mistral"
alias = "large-4"
supports_images = true
max_context_length = 1000000
max_output_tokens = 262144
{}"#,
        model.platform_enabled(),
        model.provider_toml()
    )
}

/// A supervisor over the pinned server. Only the test's model can request tools; HOME and cwd are disposable.
pub fn supervisor_config(run: &Path, model: &ScriptedModel) -> SupervisorConfig {
    let home = run.join("home");
    let vibe_home = run.join("vibe-home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&vibe_home).unwrap();
    let gitconfig = home.join(".gitconfig");
    std::fs::write(
        &gitconfig,
        "[user]\nname = Test\nemail = test@example.invalid\n[init]\ndefaultBranch = main\n",
    )
    .unwrap();
    std::fs::write(vibe_home.join("config.toml"), vibe_config(model)).unwrap();
    let mut config = SupervisorConfig::new(server_bin(), vibe_home, run.to_path_buf());
    config.home = Some(home);
    config.extra_env.extend([
        ("GIT_CONFIG_GLOBAL".to_owned(), gitconfig.into_os_string()),
        ("GIT_CONFIG_NOSYSTEM".to_owned(), "1".into()),
        ("PYTHONUNBUFFERED".to_owned(), "1".into()),
    ]);
    config.extra_env.extend(vw_keys::vibe_env(model.server()));
    if !model.platform_enabled() {
        // Model-only probes do not need a desktop keyring or its service discovery.
        config.extra_env.push((
            "PYTHON_KEYRING_BACKEND".into(),
            "keyring.backends.null.Keyring".into(),
        ));
    }
    // Match the existing public fixture's budget; no startup or request timeout is extended.
    config.handshake_timeout = std::time::Duration::from_secs(60);
    config.shutdown_grace = std::time::Duration::from_secs(1);
    config.backoff = vw_supervisor::Backoff {
        initial: std::time::Duration::from_millis(20),
        factor: 2,
        max: std::time::Duration::from_millis(200),
        max_attempts: 3,
        stable_after: std::time::Duration::from_secs(60),
    };
    config
}

/// Repoint the provider in a reused home without resetting settings the test changed before restart.
pub fn reuse_home(config: &mut SupervisorConfig, home: &Path, model: &ScriptedModel) {
    std::fs::create_dir_all(home).unwrap();
    let file = home.join("config.toml");
    if file.exists() {
        let mut parsed: toml::Value = std::fs::read_to_string(&file).unwrap().parse().unwrap();
        if let Some(providers) = parsed
            .get_mut("providers")
            .and_then(toml::Value::as_array_mut)
        {
            for provider in providers {
                if provider.get("name").and_then(toml::Value::as_str) == Some("mistral") {
                    provider["api_base"] = toml::Value::String(model.base().to_owned());
                }
            }
        }
        std::fs::write(&file, toml::to_string(&parsed).unwrap()).unwrap();
    } else {
        std::fs::write(&file, vibe_config(model)).unwrap();
    }
    config.vibe_home = home.to_path_buf();
}
