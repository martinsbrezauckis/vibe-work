//! Settings removal for one browser. Uses the owner-only signed unpair, per-browser fences and setup receipt;
//! shared helper and credential are released only after their last user is removed.
//! Depends on: signed owner unpair and setup receipts.
//! Called by: browser Settings removal commands.
use super::{PairingState, agent_browser::Browser, commands::invalid, setup};
use crate::{CommandResult, Core, CoreError};
use serde_json::json;
use std::sync::{PoisonError, atomic::Ordering};

/// A removed browser must not regain pairing facts from the other browser's shared helper.
pub(super) fn is_removed(shared: &crate::core::Shared, browser: Browser) -> bool {
    let chrome = &shared.chrome_control;
    (browser == Browser::Chrome
        || (browser == Browser::Edge && cfg!(feature = "your-edge"))
        || (browser == Browser::Firefox && cfg!(feature = "your-firefox")))
        && !browser.enabled(shared)
        && !setup::registered(
            &chrome.config,
            chrome.dir(),
            if browser == Browser::Firefox {
                "Mozilla Firefox"
            } else if browser == Browser::Edge {
                "Microsoft Edge"
            } else {
                "Google Chrome"
            },
        )
}

/// Ignore a pre-removal status reply or pairing event for a browser whose setup is now gone.
/// A switched-off browser with its registration still present retains its ordinary pairing history.
pub(super) fn filter_observation(
    shared: &crate::core::Shared,
    data: &mut serde_json::Value,
) -> bool {
    let removed: Vec<_> = [Browser::Chrome, Browser::Edge, Browser::Firefox]
        .into_iter()
        .filter(|browser| is_removed(shared, *browser))
        .collect();
    let removed_profile =
        |profile: &str| removed.iter().any(|browser| browser.owns_profile(profile));
    if data["profileId"].as_str().is_some_and(removed_profile) {
        return false;
    }
    if let Some(profiles) = data
        .get_mut("profiles")
        .and_then(serde_json::Value::as_array_mut)
    {
        profiles.retain(|profile| !profile["profileId"].as_str().is_some_and(removed_profile));
    }
    true
}

impl Core {
    pub(super) async fn remove_browser(
        &self,
        browser: Browser,
    ) -> Result<CommandResult, CoreError> {
        if browser == Browser::Edge && !cfg!(feature = "your-edge") {
            return Err(invalid(setup::absent_for(true)));
        }
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        let state = chrome
            .dir()
            .ok_or_else(|| invalid("Vibe Work has no app folder."))?;
        let name = if browser == Browser::Edge {
            "Microsoft Edge"
        } else {
            "Google Chrome"
        };
        setup::migrate_selections(&chrome.config, state).map_err(|reason| invalid(&reason))?;
        let keep_shared = setup::has_other(&chrome.config, state, name)
            || super::safari::enabled(&self.shared)
            || (cfg!(feature = "your-firefox") && state.join("firefox-window").is_file());
        let user_data = setup::user_data(&chrome.config, Some(state), name);
        let was_enabled = browser.enabled(&self.shared);
        // A dormant signed store needs the same owner key to revoke only this browser's pins.
        // A deleted credential makes every old pin unusable; removal must still offer recovery.
        if !chrome.owner.is_running()
            && state.join("paired-profiles.json").is_file()
            && !chrome.owner.credential_missing.load(Ordering::SeqCst)
            && let Err(reason) = super::owner::start_for_removal(&self.shared).await
            && !chrome.owner.credential_missing.load(Ordering::SeqCst)
        {
            return Err(invalid(&reason));
        }
        super::pairing_approval::removed_browser(chrome, browser);
        if chrome.owner.is_running() {
            let answer = chrome
                .owner
                .rpc("owner.browserUnpair", json!({"browser":browser.id()}))
                .await;
            if !answer.is_some_and(|answer| answer["unpaired"] == true) {
                chrome.owner.approvals.forget_removed(browser);
                return Err(invalid(&browser.words(
                    "The {browser} connection could not be removed. Try again.",
                )));
            }
        }
        chrome
            .set_browser_enabled(browser, false)
            .map_err(|reason| invalid(&reason))?;
        super::switches::stop_work(&self.shared, browser);
        let enrollment = if browser == Browser::Edge {
            super::enroll_browser::Browser::Edge
        } else {
            super::enroll_browser::Browser::Chrome
        };
        enrollment
            .requested(&self.shared)
            .store(false, Ordering::SeqCst);
        enrollment
            .enroll(&self.shared)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .connection_ended(enrollment);
        if !keep_shared {
            super::stop_owner(&self.shared).await;
        }
        let external = if browser == Browser::Chrome {
            super::extension_install::remove(&chrome.config, state)
                .map_err(|reason| invalid(&reason))?
        } else {
            false
        };
        setup::remove_browser(&chrome.config, state, name, keep_shared)
            .map_err(|reason| invalid(&reason))?;
        forget_browser(chrome, browser, external, user_data);
        if keep_shared {
            super::switches::deliver(&self.shared).await?;
        } else {
            if let Some(keys) = &self.shared.config.keys {
                keys.delete_chrome_owner()
                    .await
                    .map_err(|_| invalid(vw_vocabulary::texts::CHROME_OWNER_DELETE_FAILED))?;
            }
            chrome.reset();
            super::profiles::forget_all(chrome);
            // With no connection left, retain the pre-setup switch default for the next Connect.
            chrome
                .set_browser_enabled(browser, was_enabled)
                .map_err(|reason| invalid(&reason))?;
            chrome
                .owner
                .credential_missing
                .store(false, Ordering::SeqCst);
            // Remembering display facts above may have created an empty state folder after cleanup.
            setup::remove(&chrome.config, state).map_err(|reason| invalid(&reason))?;
        }
        super::owner::publish(&self.shared);
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
            hint: None,
        })
    }
}

fn forget_browser(
    chrome: &super::ChromeControl,
    browser: Browser,
    external: bool,
    user_data: Vec<std::path::PathBuf>,
) {
    if browser == Browser::Chrome {
        chrome.forget_paired();
        *chrome
            .owner
            .pairing
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = (PairingState::Unknown, None);
        chrome.removed_external.store(external, Ordering::SeqCst);
        *chrome
            .left_in_chrome
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = user_data;
    } else {
        super::edge_state::remember_pairing(chrome, false);
        *chrome
            .owner
            .edge_pairing
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = (PairingState::Unknown, None);
    }
    chrome.owner.connected_profiles.remove_browser(browser);
    super::profiles::remove_browser(chrome, browser);
    chrome
        .chat_profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|_, profile| !browser.owns_profile(profile));
    chrome
        .work
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove_browser(browser);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn observations_without_profiles_remain_unchanged() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        for mut data in [serde_json::Value::Null, json!({"unpaired":true})] {
            let before = data.clone();
            assert!(filter_observation(&fixture.core.shared, &mut data));
            assert_eq!(data, before);
        }
        fixture.finish().await;
    }

    #[cfg(not(feature = "your-edge"))]
    #[tokio::test]
    async fn optional_browser_observations_are_unchanged_without_its_module() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let mut data = json!({"profiles":[{"profileId":"edge:p","connected":true}]});
        let before = data.clone();
        assert!(filter_observation(&fixture.core.shared, &mut data));
        assert_eq!(data, before);
        fixture.finish().await;
    }
}
