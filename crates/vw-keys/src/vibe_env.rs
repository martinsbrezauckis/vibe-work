//! The `VIBE_*` variables that point every key-using Mistral call of `vibe-app-server` at the model
//! proxy. Vibe's environment config layer (`core/config/layers/environment.py`,
//! prefix `VIBE_`, JSON for lists) sits above the user and project `config.toml` layers
//! (`default_orchestrator.py:34`), so a `config.toml` (ours, the CLI's or a trusted project's) cannot
//! point the `mistral` provider elsewhere; list fields merge by `name` with the higher layer winning
//! the whole entry, so each entry is complete. Nothing is written to disk.
//!
//! | Variable | Value | Covers |
//! |---|---|---|
//! | `VIBE_PROVIDERS` | `mistral` → `<proxy>/v1` | model calls, connector gateway, telemetry, web search, identity |
//! | `VIBE_CONSOLE_BASE_URL` | `<proxy>/console` | `account/read` and experiments `/whoami` |
//! | `VIBE_VIBE_BASE_URL` | `<proxy>/chat` | managed admin config |
//! | `VIBE_TTS_PROVIDERS` | `mistral` → `<proxy>` | narration (`/v1/audio/speech`) |
//! | `VIBE_TRANSCRIBE_PROVIDERS` | `mistral` → `ws://<proxy address>` | realtime transcription: refused by the proxy (no route), so it fails on this machine instead of sending the placeholder to Mistral |
//!
//! Not covered and why: the built-in `lean` agent profile hard-codes its own provider
//! (`core/agents/models.py:139-150`) and needs `agents/install`, which the gate guards; with the
//! placeholder it fails closed at Mistral. An enforced admin config (Mistral's managed config) can
//! override layers above ours; it would also only ever carry the placeholder.
//!
//! Depends on: `serde_json`. Called by: `service`.

use std::ffi::OsString;

/// The overrides for a proxy at `base` (`http://127.0.0.1:<port>`, no trailing slash).
#[must_use]
pub fn vibe_env(base: &str) -> Vec<(String, OsString)> {
    let providers = serde_json::json!([{
        "name": "mistral",
        "api_base": format!("{base}/v1"),
        "api_key_env_var": "MISTRAL_API_KEY",
        "browser_auth_base_url": "https://console.mistral.ai",
        "browser_auth_api_base_url": "https://console.mistral.ai/api",
        "backend": "mistral"
    }]);
    let tts = serde_json::json!([{
        "name": "mistral",
        "api_base": base,
        "api_key_env_var": "MISTRAL_API_KEY"
    }]);
    let ws = base.replacen("http://", "ws://", 1);
    let transcribe = serde_json::json!([{
        "name": "mistral",
        "api_base": ws,
        "api_key_env_var": "MISTRAL_API_KEY"
    }]);
    vec![
        ("VIBE_PROVIDERS".to_owned(), providers.to_string().into()),
        (
            "VIBE_CONSOLE_BASE_URL".to_owned(),
            format!("{base}/console").into(),
        ),
        (
            "VIBE_VIBE_BASE_URL".to_owned(),
            format!("{base}/chat").into(),
        ),
        ("VIBE_TTS_PROVIDERS".to_owned(), tts.to_string().into()),
        (
            "VIBE_TRANSCRIBE_PROVIDERS".to_owned(),
            transcribe.to_string().into(),
        ),
    ]
}
