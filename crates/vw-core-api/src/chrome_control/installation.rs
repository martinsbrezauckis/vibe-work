//! Browser-local read-only extension facts for Settings and agent views. Setup receipts own folder identity;
//! live paired profiles select their version, otherwise the newest installed version is shown.
use super::{agent_browser::Browser, probe, setup};
use crate::core::Shared;
use std::path::PathBuf;

pub(super) fn profiles(shared: &Shared, browser: Browser) -> Vec<PathBuf> {
    let chrome = &shared.chrome_control;
    setup::user_data(
        chrome.dir(),
        match browser {
            Browser::Chrome => "Google Chrome",
            Browser::Edge => "Microsoft Edge",
        },
    )
    .iter()
    .flat_map(|dir| probe::profiles(dir, &chrome.config.extension_id))
    .collect()
}

pub(super) fn version(shared: &Shared, browser: Browser) -> Option<String> {
    let chrome = &shared.chrome_control;
    let live: Vec<_> = chrome
        .owner
        .connected_profiles
        .snapshot()
        .into_iter()
        .filter(|(id, _)| browser.owns_profile(id))
        .collect();
    super::addon::installed_version(&[], &chrome.config.extension_id, true, &live).or_else(|| {
        profiles(shared, browser)
            .iter()
            .filter_map(|profile| probe::installed_version(profile, &chrome.config.extension_id))
            .max_by(|a, b| {
                vw_vibe_install::support::compare_versions(a, b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    })
}

/// The same browser identity, diagnosis and version in all agent-facing add-on views.
pub(crate) fn addons(shared: &Shared) -> serde_json::Value {
    serde_json::Value::Array(
        [Browser::Chrome]
            .into_iter()
            .chain(cfg!(feature = "your-edge").then_some(Browser::Edge))
            .map(|browser| {
                let view = browser.addon(shared);
                serde_json::json!({"id":browser.id(), "name":browser.name(), "state":view.label,
                "stateCode":view.state, "label":view.label, "instruction":view.instruction,
                "extensionVersion":view.extension_version})
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn chrome_version_ignores_foreign_live_profiles_even_when_their_module_is_off() {
        let core = crate::offline_core::start("browser-version-scope").await;
        core.shared.chrome_control.owner.connected_profiles.observe(
            &serde_json::json!({"profiles":[
                {"profileId":"edge:p","connected":true,"extensionVersion":"0.1.4"},
                {"profileId":"firefox:p","connected":true,"extensionVersion":"0.1.9"}
            ]}),
        );
        assert_eq!(version(&core.shared, Browser::Chrome), None);
        for id in ["p", "chrome:p"] {
            core.shared.chrome_control.owner.connected_profiles.observe(
                &serde_json::json!({"profiles":[
                    {"profileId":id,"connected":true,"extensionVersion":"0.1.3"},
                    {"profileId":"edge:p","connected":true,"extensionVersion":"0.1.1"},
                    {"profileId":"firefox:p","connected":true,"extensionVersion":"0.1.0"}
                ]}),
            );
            assert_eq!(
                version(&core.shared, Browser::Chrome).as_deref(),
                Some("0.1.3")
            );
        }
        core.shutdown().await;
    }
}
