//! Network Test's second handshake, using the installed Vibe's Python and TLS/proxy code, with the trust store
//! Vibe's config selects (`enable_system_trust_store`), so the Test and Vibe's own connections agree.
//! Called after the Rust client succeeds. No model key enters the isolated helper or its output.
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use vw_mistral::network::{KEYS, NetworkSettings};

use crate::network_view::{NetworkTestKind, NetworkTestView};

pub(crate) async fn verify(
    shared: &crate::core::Shared,
    settings: &NetworkSettings,
    view: &mut NetworkTestView,
) {
    let url = &shared.config.network_test_url;
    if !view.reached || !url.starts_with("https://") {
        return;
    }
    let checked = check(shared, settings, url).await;
    match checked.as_ref().and_then(|v| v["kind"].as_str()) {
        Some("reached") => {}
        Some("certificate") => {
            view.reached = false;
            view.kind = NetworkTestKind::CertificateNotTrusted;
            view.summary = checked
                .as_ref()
                .and_then(|v| v["reason"].as_str())
                .map_or_else(
                    || "Vibe 3.0 will refuse this certificate under Python 3.14's certificate checks.".to_owned(),
                    |reason| format!("Vibe 3.0 will refuse this certificate because {reason}."),
                );
            view.next_step = Some(
                "Ask your IT department for a certificate chain that meets these checks.".into(),
            );
        }
        _ => {
            view.reached = false;
            view.kind = NetworkTestKind::Failed;
            view.summary =
                "Vibe Work connected, but Vibe 3.0's connection could not be verified.".into();
            view.next_step =
                Some("Check that Vibe is installed, then try the connection test again.".into());
        }
    }
}

async fn check(
    shared: &crate::core::Shared,
    settings: &NetworkSettings,
    url: &str,
) -> Option<Value> {
    let python = shared.plugin_approvals.vibe_python()?;
    let values: serde_json::Map<String, Value> = KEYS
        .iter()
        .filter_map(|key| settings.get(key).map(|value| ((*key).into(), value.into())))
        .collect();
    let system_trust_store = system_trust_store(&shared.instructions.own_home.join("config.toml"));
    let input = serde_json::to_vec(
        &json!({"url":url, "settings":values, "systemTrustStore": system_trust_store}),
    )
    .ok()?;
    let mut command = tokio::process::Command::new(python);
    command.env_clear();
    for name in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command
        .args(["-I", "-B", "-c", include_str!("network_python.py")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let operation = async {
        child.stdin.take()?.write_all(&input).await.ok()?;
        let output = child.wait_with_output().await.ok()?;
        if !output.status.success() || output.stdout.len() > 1024 {
            return None;
        }
        serde_json::from_slice(&output.stdout).ok()
    };
    tokio::time::timeout(Duration::from_secs(20), operation)
        .await
        .ok()
        .flatten()
}

/// Vibe's own `enable_system_trust_store` from its config (Vibe's default, false, when the key is absent or the file
/// cannot be read), so the helper builds the same TLS context the server pinned at start: the operating system's
/// store (`truststore`) or the bundled certifi file. Vibe Work writes `true` at launch (`vw_host::trust_store`).
fn system_trust_store(config: &std::path::Path) -> bool {
    std::fs::read_to_string(config)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|table| table.get("enable_system_trust_store")?.as_bool())
        .unwrap_or(false)
}
