//! Helpers for the core API and Debug API tests: a `Core` over the fake server in `--script`
//! mode (with a canary API key and a method log), and a Debug API whose token is read from the
//! descriptor file exactly as an external agent would.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use vw_core_api::{Core, CoreConfig};
use vw_debug_api::{DebugApi, DebugApiConfig};
use vw_supervisor::{ApiKey, SecretError, SecretProvider};

use super::fake::{fake_config, scratch};
use super::gate::FakeConfirmer;
use super::http::Client;

/// A fake API key; it must never appear in any Debug API output.
pub const CANARY_KEY: &str = "sk-vw-canary-7f3a9c1e5b2d4086";

/// Supplies [`CANARY_KEY`].
pub struct CanaryKey;

impl SecretProvider for CanaryKey {
    fn mistral_api_key(&self) -> Result<Option<ApiKey>, SecretError> {
        Ok(Some(ApiKey::new(CANARY_KEY)))
    }
}

/// A core over the scripted fake server.
pub struct ScriptRig {
    pub core: Core,
    pub dir: PathBuf,
    pub log: PathBuf,
    pub confirmer: FakeConfirmer,
}

impl ScriptRig {
    /// Methods the fake server received, in order (`initialize` excluded).
    pub fn received(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

/// Starts a core on `fake_vibe_server --script` with the canary key; `extra` adds server flags.
/// The fake also writes the key to its stderr at start (`--stderr-key-at-start`), so the
/// redaction of stderr shown by `/server` is exercised.
pub async fn script_core(name: &str, extra: &[&str]) -> ScriptRig {
    script_core_with(name, extra, |_| {}).await
}

/// [`script_core`] with the core config adjusted by `tweak`.
pub async fn script_core_with(
    name: &str,
    extra: &[&str],
    tweak: impl FnOnce(&mut CoreConfig),
) -> ScriptRig {
    let dir = scratch(name);
    let log = dir.join("methods.log");
    let log_text = log.to_string_lossy().into_owned();
    let mut args = vec![
        "--script",
        "--log-methods",
        &log_text,
        "--stderr-key-at-start",
    ];
    args.extend_from_slice(extra);
    let mut config = fake_config(name, &args);
    config.cwd = dir.clone();
    config.vibe_home = dir.join("vibe-home");
    let confirmer = FakeConfirmer::new();
    let mut core_config = CoreConfig::default();
    tweak(&mut core_config);
    let core = Core::start(config, Arc::new(CanaryKey), confirmer.clone(), core_config)
        .await
        .expect("core starts on the fake server");
    ScriptRig {
        core,
        dir,
        log,
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
