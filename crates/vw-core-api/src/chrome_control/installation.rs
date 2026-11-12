//! Browser-wide installation facts from read-only probes. Folder availability never proves a pairing UUID;
//! profile selection and history use authenticated helper identities, including when a browser is reopened.
use super::{agent_browser::Browser, probe, setup};
use crate::core::Shared;
use std::path::PathBuf;

pub(super) fn profiles(shared: &Shared, browser: Browser) -> Vec<PathBuf> {
    let chrome = &shared.chrome_control;
    let installed = installed_profiles(chrome, browser);
    let selectable = selectable_profiles(chrome, browser, &installed);
    if selectable.is_empty() {
        installed
    } else {
        selectable
    }
}

/// Disabled unpaired copies are installation prompts, not browser choices. Unknown reads stay ambiguous.
fn selectable_profiles(
    chrome: &super::ChromeControl,
    browser: Browser,
    installed: &[PathBuf],
) -> Vec<PathBuf> {
    let prober = if browser == Browser::Edge {
        &chrome.edge_prober
    } else {
        &chrome.prober
    };
    installed
        .iter()
        .filter(|profile| {
            !matches!(
                prober.probe_for(
                    &chrome.config,
                    vec![(*profile).clone()],
                    &chrome.config.extension_id,
                    std::time::Instant::now()
                ),
                probe::Probe::Off | probe::Probe::AwaitingEnable
            )
        })
        .cloned()
        .collect()
}

fn installed_profiles(chrome: &super::ChromeControl, browser: Browser) -> Vec<PathBuf> {
    setup::user_data(
        &chrome.config,
        chrome.dir(),
        match browser {
            Browser::Chrome => "Google Chrome",
            Browser::Edge => "Microsoft Edge",
            Browser::Safari | Browser::Firefox => return Vec::new(),
        },
    )
    .iter()
    .flat_map(|dir| probe::profiles_for(&chrome.config, dir, &chrome.config.extension_id))
    .collect()
}

fn directory_record(chrome: &super::ChromeControl, browser: Browser) -> Option<PathBuf> {
    let dir = chrome.dir()?;
    Some(match browser {
        Browser::Chrome => dir.join("profile-directory.json"),
        Browser::Edge => dir.join("edge/profile-directory.json"),
        _ => return None,
    })
}

/// Keep legacy history UUIDs without recording an unproven launch directory.
pub(super) fn remember_directory(chrome: &super::ChromeControl, browser: Browser) {
    super::profile_folders::observe(chrome, browser);
}

pub(super) fn forget_directory(chrome: &super::ChromeControl, browser: Browser) {
    super::profile_folders::forget(chrome, browser);
    if let Some(record) = directory_record(chrome, browser) {
        let _ = std::fs::remove_file(record);
    }
}

/// The frozen hello cannot prove a filesystem directory. Normal browser startup may reconnect a trusted
/// identity, but only the subsequent authenticated inventory can authorize a task for the selected profile.
pub(super) fn paired_directory(_shared: &Shared, _browser: Browser) -> Option<String> {
    None
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
    super::addon::installed_version(
        &chrome.config,
        &[],
        &chrome.config.extension_id,
        true,
        &live,
    )
    .or_else(|| {
        profiles(shared, browser)
            .iter()
            .filter_map(|profile| {
                probe::version_for(&chrome.config, profile, &chrome.config.extension_id)
            })
            .max_by(|a, b| {
                vw_vibe_install::support::compare_versions(a, b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    })
}

/// Only a live extension supplies macOS profile identities; no browser-owned file is inspected.
pub(super) fn reported_profiles(shared: &Shared, browser: Browser) -> Vec<String> {
    shared
        .chrome_control
        .owner
        .connected_profiles
        .snapshot()
        .into_iter()
        .filter(|(id, _)| browser.owns_profile(id))
        .map(|(id, _)| id)
        .collect()
}

pub(super) fn observed_state(
    config: &setup::ChromeControlConfig,
    state: super::AddonState,
) -> super::AddonState {
    if config.is_macos() && state == super::AddonState::ExtensionNotInstalled {
        super::AddonState::NotConnected
    } else {
        state
    }
}

/// The same browser identity, diagnosis and version in all agent-facing add-on views.
pub(crate) fn addons(shared: &Shared) -> serde_json::Value {
    serde_json::Value::Array(
        [Browser::Chrome]
            .into_iter()
            .chain(cfg!(feature = "your-edge").then_some(Browser::Edge))
            .chain(cfg!(feature = "your-firefox").then_some(Browser::Firefox))
            .map(|browser| {
                let view = browser.addon(shared);
                let mut fact = serde_json::json!({"id":browser.id(), "name":browser.name(), "state":view.label,
                "stateCode":view.state, "label":view.label, "instruction":view.instruction,
                "extensionVersion":view.extension_version});
                if browser == Browser::Firefox && let Some(sharing) = super::firefox_sharing::shown(shared) { fact["sharing"] = serde_json::to_value(sharing).unwrap_or_default(); }
                fact
            })
            .chain(super::safari::agent_fact(shared))
            .collect(),
    )
}

/// On macOS pairing, not an on-disk installation probe, establishes the successful connection.
pub(super) fn present(config: &setup::ChromeControlConfig, view: &mut super::AddonView) {
    if config.is_macos() && view.state == super::AddonState::Connected {
        view.label = "Paired".into();
    }
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
