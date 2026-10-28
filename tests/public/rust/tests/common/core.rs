//! Real-Vibe and no-Vibe core fixtures, with a canary key and a passive request recorder.
//! Depends on the pinned executable, loopback scripted models and disposable files; called by integration tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use vw_core_api::{Core, CoreConfig};
use vw_debug_api::{DebugApi, DebugApiConfig};
use vw_supervisor::{ApiKey, SecretError, SecretProvider};

use super::gate::FakeConfirmer;
use super::http::Client;
use super::processes::scratch;

#[path = "test_keyring.rs"]
pub mod test_keyring;

/// A fake API key; it must never appear in any Debug API output.
pub const CANARY_KEY: &str = "sk-vw-canary-7f3a9c1e5b2d4086";

/// Supplies [`CANARY_KEY`].
pub struct CanaryKey;

impl SecretProvider for CanaryKey {
    fn mistral_api_key(&self) -> Result<Option<ApiKey>, SecretError> {
        Ok(Some(ApiKey::new(CANARY_KEY)))
    }
}

/// A core and its real request log (empty when the test explicitly needs no Vibe).
pub struct ScriptRig {
    pub core: Core,
    pub dir: PathBuf,
    pub log: PathBuf,
    pub confirmer: FakeConfirmer,
}

impl ScriptRig {
    /// The real ID captured while provisioning a named fixture session.
    pub fn id(&self, name: &str) -> String {
        let record: Value =
            serde_json::from_slice(&std::fs::read(self.dir.join("seed-ids.json")).unwrap())
                .unwrap();
        record["ids"][name]
            .as_str()
            .unwrap_or_else(|| panic!("no real session for {name}"))
            .to_owned()
    }

    /// The real folder used for a named stored session.
    pub fn folder(&self, name: &str) -> PathBuf {
        let record: Value =
            serde_json::from_slice(&std::fs::read(self.dir.join("seed-ids.json")).unwrap())
                .unwrap();
        PathBuf::from(record["folders"][name].as_str().unwrap())
    }

    /// The loopback model/platform endpoint selected for this fixture.
    pub fn model_server(&self) -> String {
        std::fs::read_to_string(self.dir.join(".model-server")).unwrap()
    }

    /// The actual disposable Vibe home, including a home shared across app restarts.
    pub fn vibe_home(&self) -> PathBuf {
        std::fs::read_to_string(self.dir.join(".vibe-home"))
            .map_or_else(|_| self.dir.join("vibe-home"), PathBuf::from)
    }

    /// Real session IDs returned before local ownership bookkeeping can fail.
    pub fn opened_ids(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("responses.jsonl"))
            .unwrap()
            .lines()
            .filter_map(|line| {
                let frame: Value = serde_json::from_str(line).unwrap();
                frame
                    .get("message")
                    .unwrap_or(&frame)
                    .pointer("/result/state/session/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect()
    }

    /// Methods the real server received, in order (`initialize` excluded).
    pub fn received(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

/// Starts the real pinned Vibe with a local text model; protocol responses are never scripted.
pub async fn script_core(name: &str, extra: &[&str]) -> ScriptRig {
    script_core_with(name, extra, |_| {}).await
}

/// [`script_core`] with the core config adjusted by `tweak`.
pub async fn script_core_with(
    name: &str,
    extra: &[&str],
    tweak: impl FnOnce(&mut CoreConfig),
) -> ScriptRig {
    script_core_with_home(name, extra, None, tweak).await
}

/// Runs separate cores against the same test Vibe home while their app data folders differ.
pub async fn script_core_with_home(
    name: &str,
    extra: &[&str],
    home: Option<&Path>,
    tweak: impl FnOnce(&mut CoreConfig),
) -> ScriptRig {
    real_core_with_server(
        name,
        extra,
        home,
        super::shared_real_vibe::text_script(),
        tweak,
        |_| {},
    )
    .await
    .0
}

/// Custom model answers, with real turns, tool callbacks and persistence owned by Vibe.
pub async fn real_core(
    name: &str,
    extra: &[&str],
    home: Option<&Path>,
    script: super::scripted_model::Script,
    tweak: impl FnOnce(&mut CoreConfig),
) -> (ScriptRig, super::scripted_model::ScriptedModel) {
    real_core_with_server(name, extra, home, script, tweak, |_| {}).await
}

/// A fixture whose launcher can inject a real process fault or adjust its actual configuration.
pub async fn real_core_with_server(
    name: &str,
    extra: &[&str],
    home: Option<&Path>,
    script: super::scripted_model::Script,
    tweak: impl FnOnce(&mut CoreConfig),
    server: impl FnOnce(&mut vw_supervisor::SupervisorConfig),
) -> (ScriptRig, super::scripted_model::ScriptedModel) {
    let model = super::scripted_model::ScriptedModel::start(script).await;
    let rig = core_on_model(name, extra, home, &model, tweak, server).await;
    (rig, model)
}

async fn core_on_model(
    name: &str,
    extra: &[&str],
    home: Option<&Path>,
    model: &super::scripted_model::ScriptedModel,
    tweak: impl FnOnce(&mut CoreConfig),
    server: impl FnOnce(&mut vw_supervisor::SupervisorConfig),
) -> ScriptRig {
    assert!(
        extra.is_empty(),
        "{name}: real Vibe accepts no synthetic fixture options: {extra:?}"
    );
    let dir = scratch(name);
    let mut config = super::shared_real_vibe::supervisor_config(&dir, model);
    if let Some(home) = home {
        super::shared_real_vibe::reuse_home(&mut config, home, model);
    }
    let log = dir.join("methods.log");
    record(&mut config, &dir);
    server(&mut config);
    if model.platform_enabled() {
        test_keyring::install(&mut config);
    }
    std::fs::write(
        dir.join(".vibe-home"),
        config.vibe_home.to_string_lossy().as_bytes(),
    )
    .unwrap();
    std::fs::write(dir.join(".model-server"), model.server()).unwrap();
    let confirmer = FakeConfirmer::new();
    let mut core_config = CoreConfig::default();
    tweak(&mut core_config);
    if let Some(tools) = &core_config.agent_tools {
        std::fs::write(
            config.vibe_home.join("hooks.toml"),
            vw_host::agent_tools::hook_block(&tools.binary, &tools.endpoint_file()),
        )
        .unwrap();
    }
    let secret = model.route(&mut config).await;
    let core = Core::start(config, secret, confirmer.clone(), core_config)
        .await
        .expect("core starts on the pinned real Vibe");
    assert_eq!(
        core.status().vibe_version.as_deref(),
        Some(super::shared_real_vibe::PIN.trim())
    );
    ScriptRig {
        core,
        dir,
        log,
        confirmer,
    }
}

/// Provision stored sessions with real turns before starting the core under test; retain captured aliases on restart.
pub async fn stored_core_with(
    name: &str,
    home: Option<&Path>,
    items: &[Value],
    script: super::scripted_model::Script,
    tweak: impl FnOnce(&mut CoreConfig),
) -> ScriptRig {
    stored_core_with_server(name, home, items, script, tweak, |_| {}).await
}

/// Stored sessions with an OS/filesystem fault applied only to the core under test.
pub async fn stored_core_with_server(
    name: &str,
    home: Option<&Path>,
    items: &[Value],
    script: super::scripted_model::Script,
    tweak: impl FnOnce(&mut CoreConfig),
    server: impl FnOnce(&mut vw_supervisor::SupervisorConfig),
) -> ScriptRig {
    let setup = scratch(&format!("{name}-setup"));
    let home = home.map_or_else(|| setup.join("vibe-home"), Path::to_path_buf);

    let captures = home.join("fixture-session-ids.json");
    let provisioned = !captures.is_file();
    let record: Value = if captures.is_file() {
        serde_json::from_slice(&std::fs::read(&captures).unwrap()).unwrap()
    } else {
        let seed_model =
            super::scripted_model::ScriptedModel::start(super::shared_real_vibe::text_script())
                .await;
        let record = super::seed_sessions::create(&setup, &home, &seed_model, items).await;
        std::fs::write(&captures, serde_json::to_vec(&record).unwrap()).unwrap();
        record
    };
    let model = super::scripted_model::ScriptedModel::start(script).await;
    if provisioned && (model.platform_enabled() || model.registry().is_some()) {
        let file = home.join("config.toml");
        let mut config: toml::Value = std::fs::read_to_string(&file).unwrap().parse().unwrap();
        config["enable_connectors"] = toml::Value::Boolean(model.platform_enabled());
        std::fs::write(file, toml::to_string(&config).unwrap()).unwrap();
    }
    let rig = core_on_model(name, &[], Some(&home), &model, tweak, server).await;
    std::fs::write(
        rig.dir.join("seed-ids.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    rig
}

/// Record bytes sent to the real process without answering, rewriting or dropping any of them.
pub fn record(config: &mut vw_supervisor::SupervisorConfig, dir: &Path) {
    let mut tap = vec![
        PathBuf::from(env!("CARGO_BIN_EXE_vibe_wire_tap")).into_os_string(),
        "--methods".into(),
        dir.join("methods.log").into_os_string(),
        "--channels".into(),
        dir.join("channels.log").into_os_string(),
        "--params".into(),
        dir.join("params.jsonl").into_os_string(),
        "--requests".into(),
        dir.join("requests.jsonl").into_os_string(),
        "--responses".into(),
        dir.join("responses.jsonl").into_os_string(),
        "--launch".into(),
        dir.join("launch.json").into_os_string(),
        "--agent-config".into(),
        dir.join("agent-config.jsonl").into_os_string(),
        "--plugin-roots".into(),
        config.vibe_home.join("plugin-roots.jsonl").into_os_string(),
        "--".into(),
    ];
    tap.append(&mut config.launcher);
    config.launcher = tap;
}

/// A deliberately absent program for fixtures whose CoreConfig has start_vibe=false.
pub fn local_server(dir: &Path) -> vw_supervisor::SupervisorConfig {
    vw_supervisor::SupervisorConfig::new(
        dir.join("no-server-requested"),
        dir.join("vibe-home"),
        dir.to_path_buf(),
    )
}

/// A local core with no server or model, and proof that no Vibe PID was started.
pub async fn no_vibe_core(name: &str, extra: &[&str]) -> ScriptRig {
    no_vibe_core_with(name, extra, |_| {}).await
}

pub async fn no_vibe_core_with(
    name: &str,
    extra: &[&str],
    tweak: impl FnOnce(&mut CoreConfig),
) -> ScriptRig {
    assert!(
        extra.is_empty(),
        "a core without Vibe accepts no server options"
    );
    let dir = scratch(name);
    let server = vw_supervisor::SupervisorConfig::new(
        dir.join("no-server-requested"),
        dir.join("vibe-home"),
        dir.clone(),
    );
    let confirmer = FakeConfirmer::new();
    let mut config = CoreConfig::default();
    tweak(&mut config);
    config.start_vibe = false;
    let core = Core::start(server, Arc::new(CanaryKey), confirmer.clone(), config)
        .await
        .unwrap();
    assert!(core.status().server.pid.is_none());
    ScriptRig {
        core,
        dir: dir.clone(),
        log: dir.join("no-server-methods.log"),
        confirmer,
    }
}

/// Reads a descriptor file and then the token file it names, the way an agent does. The
/// descriptor itself must not carry the token.
pub fn read_descriptor(path: &Path) -> (u16, String) {
    let text = std::fs::read_to_string(path).unwrap();
    let json: Value = serde_json::from_str(&text).unwrap();
    assert!(
        json.get("token").is_none(),
        "the descriptor must not hold the token"
    );
    let port = u16::try_from(json["port"].as_u64().unwrap()).unwrap();
    let token_file = Path::new(json["tokenFile"].as_str().unwrap());
    (
        port,
        std::fs::read_to_string(token_file)
            .unwrap()
            .trim()
            .to_owned(),
    )
}

/// Starts the Debug API (forced on, as in a user build with the setting on) for `core`, with its
/// descriptor under `dir/debug-api`, and returns a client built from the descriptor.
pub async fn debug_api(core: &Core, dir: &Path) -> (DebugApi, Client) {
    let mut config = DebugApiConfig::new(dir.join("debug-api"));
    config.user_setting = Some(true);
    let api = DebugApi::start(core.clone(), config)
        .await
        .expect("debug api starts")
        .expect("enabled");
    let (port, token) = read_descriptor(api.descriptor_path());
    assert_eq!(port, api.port());
    (api, Client::new(port, &token))
}
