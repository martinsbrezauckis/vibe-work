//! The setup card's view for one browser: registration state, the package, profiles and the repair check.
use super::{
    ABSENT, ChromeControlConfig, ChromeSetupView, Path, SetupState, packaged_helper, problem,
    profiles_with, read_receipt, registration, repair_registrations, selection, store_url,
    user_data,
};

/// The status Settings shows.
pub(crate) fn view(
    config: &ChromeControlConfig,
    state: Option<&Path>,
    vibe_home: &Path,
) -> ChromeSetupView {
    view_for(config, state, vibe_home, "Google Chrome")
}

pub(in crate::chrome_control) fn view_for(
    config: &ChromeControlConfig,
    state: Option<&Path>,
    _vibe_home: &Path,
    browser: &str,
) -> ChromeSetupView {
    if !config.available() {
        return ChromeSetupView {
            state: SetupState::Absent,
            package: false,
            browsers: Vec::new(),
            profiles: Vec::new(),
            store_url: String::new(),
            store_review: false,
            pairing_state: super::super::PairingState::Unknown,
            pairing: ABSENT.to_owned(),
            fingerprint: None,
            browser_issue: None,
            install_state: None,
            states: None,
            addon: Some(super::super::addon::view(
                super::super::addon::AddonState::Unavailable,
                ABSENT,
            )),
            extension_left: None,
            extension_restart: None,
            profile_names_refused: None,
        };
    }
    let receipt = state
        .and_then(|state| read_receipt(config, state))
        .filter(|receipt| {
            receipt.browsers.iter().any(|name| name == browser)
                && state.is_some_and(|state| selection::selected(config, state, receipt, browser))
        });
    let profiles: Vec<String> = user_data(config, state, browser)
        .iter()
        .flat_map(|dir| profiles_with(config, dir, &config.extension_id))
        .collect();
    let package = packaged_helper(config);
    let blocked = problem::blocked(config, state, browser);
    let repair_failed = blocked
        || package.is_ok()
            && state.is_some_and(|state| needs_repair(config, state, browser, receipt.is_some()));
    ChromeSetupView {
        state: match (&receipt, profiles.is_empty()) {
            (Some(_), _) if repair_failed => SetupState::RepairRequired,
            (None, _) => SetupState::NotSetUp,
            (Some(_), true) => SetupState::WaitingForExtension,
            (Some(_), false) => SetupState::ExtensionAdded,
        },
        package: package.is_ok(),
        browsers: receipt
            .map(|r| {
                r.browsers
                    .into_iter()
                    .filter(|name| name == browser)
                    .collect()
            })
            .unwrap_or_default(),
        profiles,
        store_url: store_url(&config.extension_id),
        store_review: false,
        pairing_state: super::super::PairingState::Unknown,
        pairing: if repair_failed {
            "Helper setup required".into()
        } else {
            package
                .err()
                .unwrap_or_else(|| vw_vocabulary::texts::CHROME_NOT_CONNECTED.to_owned())
        },
        fingerprint: None,
        browser_issue: match browser {
            "Google Chrome" => super::super::browser::issue(config),
            "Microsoft Edge" => {
                super::super::browser::issue_for(config, vw_platform::browser::Browser::Edge)
            }
            _ => None,
        },
        install_state: if config.is_macos() {
            Some("store".into())
        } else {
            (browser == "Google Chrome")
                .then(|| {
                    state.and_then(|state| super::super::extension_install::status(config, state))
                })
                .flatten()
        },
        states: None,
        // A refused attempt stays visible without retrying it on every status poll.
        addon: (blocked || problem::blocked(config, state, browser)).then(|| {
            super::super::addon::view_for(
                super::super::addon::AddonState::AccessBlocked,
                "",
                if browser == "Microsoft Edge" {
                    super::super::agent_browser::Browser::Edge
                } else {
                    super::super::agent_browser::Browser::Chrome
                },
            )
        }),
        extension_left: None,
        extension_restart: None,
        profile_names_refused: None,
    }
}

pub(super) fn needs_repair(
    config: &ChromeControlConfig,
    state: &Path,
    browser: &str,
    receipted: bool,
) -> bool {
    if config.is_macos() {
        // Every command renders both cards. A status read must not touch the other browser.
        receipted
            && (problem::repair_failed(config, state, browser)
                || registration::check_helper(config, state).is_err())
    } else {
        repair_registrations(config, state, Some(browser)).is_err()
    }
}
