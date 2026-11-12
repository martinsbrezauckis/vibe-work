//! Read-only managed Edge policy refusals for setup. Depends on local OS policy files; never changes policy.
use serde_json::Value;

pub(super) const BLOCKED: &str = "Blocked by your organization";

fn has(value: &Value, entry: &str) -> bool {
    value
        .as_array()
        .is_some_and(|rows| rows.iter().any(|row| row == entry))
}
fn blocked(policy: &Value, extension: &str) -> bool {
    let host = super::setup::HOST_NAME;
    let host_denied = (has(&policy["NativeMessagingBlocklist"], "*")
        || has(&policy["NativeMessagingBlocklist"], host))
        && !has(&policy["NativeMessagingAllowlist"], host);
    let settings = &policy["ExtensionSettings"];
    let mode = settings[extension]
        .get("installation_mode")
        .or_else(|| settings["*"].get("installation_mode"));
    let extension_denied = if let Some(mode) = mode {
        matches!(mode.as_str(), Some("blocked" | "removed"))
    } else {
        (has(&policy["ExtensionInstallBlocklist"], "*")
            || has(&policy["ExtensionInstallBlocklist"], extension))
            && !has(&policy["ExtensionInstallAllowlist"], extension)
    };
    policy["NativeMessagingUserLevelHosts"] == false || host_denied || extension_denied
}

/// Local policy is diagnostic only; Edge remains the authority. Scratch homes never inspect the person's policy.
pub(super) fn refusal(config: &super::setup::ChromeControlConfig) -> Option<&'static str> {
    if !cfg!(feature = "your-edge") {
        return None;
    }
    #[cfg(test)]
    if let Some(policy) = &config.edge_policy {
        return blocked(policy, &config.extension_id).then_some(BLOCKED);
    }
    if config.user_home.is_some() {
        return None;
    }
    policy()
        .iter()
        .any(|policy| blocked(policy, &config.extension_id))
        .then_some(BLOCKED)
}

fn policy() -> Vec<Value> {
    // Polling Settings must not start OS readers repeatedly. Policy changes are observed on the next read interval.
    use std::sync::{Mutex, OnceLock, PoisonError};
    use std::time::{Duration, Instant};
    type Snapshot = Option<(Instant, Vec<Value>)>;
    static CACHE: OnceLock<Mutex<Snapshot>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some((when, values)) = &*cache
        && when.elapsed() < Duration::from_secs(3)
    {
        return values.clone();
    }
    let values = platform::read();
    *cache = Some((Instant::now(), values.clone()));
    values
}

use vw_platform::edge_policy as platform;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const ID: &str = super::super::setup::EXTENSION_ID;
    #[test]
    fn host_and_install_denials_are_plain_refusals() {
        for policy in [
            json!({"NativeMessagingBlocklist":["*"]}),
            json!({"NativeMessagingUserLevelHosts":false}),
            json!({"ExtensionSettings":{ID:{"installation_mode":"blocked"}}}),
            json!({"ExtensionInstallBlocklist":[ID]}),
        ] {
            assert!(blocked(&policy, ID), "{policy}");
        }
    }
    #[test]
    fn explicit_allowlists_and_unrelated_policy_do_not_claim_a_denial() {
        assert!(!blocked(
            &json!({"NativeMessagingBlocklist":["*"],"NativeMessagingAllowlist":[super::super::setup::HOST_NAME]}),
            ID
        ));
        assert!(!blocked(
            &json!({"ExtensionSettings":{"*":{"installation_mode":"blocked"},ID:{"installation_mode":"allowed"}}}),
            ID
        ));
        assert!(!blocked(
            &json!({"ExtensionInstallBlocklist":["*"],"ExtensionInstallAllowlist":[ID]}),
            ID
        ));
        assert!(!blocked(
            &json!({"HomepageLocation":"https://example.test/"}),
            ID
        ));
    }
}
