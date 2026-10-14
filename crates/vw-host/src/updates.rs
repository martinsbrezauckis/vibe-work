//! Update channel rules and the persisted status: decides from the build's
//! `plugins.updater` config whether this build may check for updates at all, and records what the last update step
//! did (`updates.json` in the data folder). The flow itself is [`crate::update_flow`].
//!
//! **Fail closed.** `tauri.conf.json` carries no updater config; only a release (or test) build adds one
//! through a `tauri build --config` overlay. [`channel`] accepts it only when:
//! - `pubkey` is a non-empty string (the plugin verifies every download against it: minisign);
//! - there is at least one endpoint, every endpoint is `https`, except that `http` is allowed for
//!   **loopback hosts only** and only when the config also sets `dangerousInsecureTransportProtocol`
//!   (the local fake update server of the tests);
//! - `requireSignedVersion` is `true` (the signature must carry the version the endpoint announces, so a
//!   replayed older release cannot be offered as newer).
//!
//! Anything else returns [`UpdatesOff`]: the app then does not register the updater plugin and makes no
//! update request.
//!
//! Depends on: serde, `serde_json`, url. Called by: `app` (`updater.rs`), the private tests.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

/// Why this build does not check for updates.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdatesOff {
    /// The build has no updater config (every development build and every build without a release
    /// overlay).
    #[error("no update channel in this build")]
    NotConfigured,
    /// The config is present but not acceptable; the reason says which rule failed.
    #[error("update channel refused: {0}")]
    Invalid(String),
}

/// An accepted update channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateChannel {
    /// Where the plugin asks for the newest version.
    pub endpoints: Vec<Url>,
}

impl UpdateChannel {
    /// The address of another file published next to the first endpoint's `latest.json` (the signed
    /// supported-Vibe manifest `vibe-support.json`). `None` when that endpoint uses the updater's `{{…}}` template
    /// variables (no fixed folder) or `name` does not join.
    #[must_use]
    pub fn sibling(&self, name: &str) -> Option<Url> {
        let first = self.endpoints.first()?;
        if first.as_str().contains("%7B%7B") || first.as_str().contains("{{") {
            return None;
        }
        first.join(name).ok()
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

/// Checks the build's `plugins.updater` value (`None` or JSON `null` when absent).
///
/// # Errors
/// [`UpdatesOff::NotConfigured`] when absent, [`UpdatesOff::Invalid`] when a rule above fails.
pub fn channel(config: Option<&Value>) -> Result<UpdateChannel, UpdatesOff> {
    let Some(config) = config.filter(|c| !c.is_null()) else {
        return Err(UpdatesOff::NotConfigured);
    };
    let invalid = |why: &str| Err(UpdatesOff::Invalid(why.to_owned()));
    let pubkey = config.get("pubkey").and_then(Value::as_str).unwrap_or("");
    if pubkey.trim().is_empty() {
        return invalid("no pubkey");
    }
    if config.get("requireSignedVersion").and_then(Value::as_bool) != Some(true) {
        return invalid("requireSignedVersion must be true");
    }
    let insecure = config
        .get("dangerousInsecureTransportProtocol")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(list) = config.get("endpoints").and_then(Value::as_array) else {
        return invalid("no endpoints");
    };
    let mut endpoints = Vec::new();
    for item in list {
        let Some(text) = item.as_str() else {
            return invalid("an endpoint is not a string");
        };
        let Ok(url) = Url::parse(text) else {
            return invalid("an endpoint is not a URL");
        };
        match url.scheme() {
            "https" => {}
            "http" if insecure && is_loopback(&url) => {}
            "http" => {
                return invalid("http endpoint outside loopback or without the insecure flag");
            }
            _ => return invalid("endpoint scheme is not https"),
        }
        endpoints.push(url);
    }
    if endpoints.is_empty() {
        return invalid("no endpoints");
    }
    Ok(UpdateChannel { endpoints })
}

/// What the last update step did, persisted as `updates.json` (no secret, no URL with credentials). Also
/// remembers which versions already caused an OS notification, so a restart does not notify again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// Unix seconds of the step.
    pub checked_at: u64,
    /// The running version.
    pub current: String,
    /// `upToDate`, `available`, `ready`, `restarting` or `failed`.
    pub outcome: String,
    /// The version concerned (`available`, `ready`, `restarting`, sometimes `failed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The failure text (`failed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The last version an "available" OS notification was shown for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notified_available: Option<String>,
    /// The last version a "ready to install" OS notification was shown for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notified_ready: Option<String>,
}

impl UpdateStatus {
    /// A status for a run that has not checked yet.
    #[must_use]
    pub fn new(current: &str) -> Self {
        Self {
            checked_at: 0,
            current: current.to_owned(),
            outcome: "none".to_owned(),
            version: None,
            error: None,
            notified_available: None,
            notified_ready: None,
        }
    }

    /// Writes the status atomically (temp file, then rename).
    ///
    /// # Errors
    /// The I/O error.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&temp, path)
    }
}
