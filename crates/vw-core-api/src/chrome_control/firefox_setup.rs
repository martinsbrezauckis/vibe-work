//! Explicit Firefox add-on registration and removal. Uses the shared helper receipt and person-confirmed offers.
use super::{agent_browser::Browser, commands::invalid, setup};
use crate::install_offers::{OfferKey, ToolsInstallOffer};
use crate::{CommandResult, Core, CoreError};
use serde_json::json;

const KIND: &str = "chromeControl";
const NAME: &str = "Mozilla Firefox";

impl Core {
    pub(super) fn firefox_setup_offer(&self) -> Result<ToolsInstallOffer, CoreError> {
        setup::packaged_helper(&self.shared.chrome_control.config).map_err(|e| invalid(&e))?;
        self.shared.install_offers.make(
            OfferKey::new(KIND, setup::PLUGIN, "firefox"),
            ToolsInstallOffer {
                offer_id: String::new(), kind: KIND.into(), name: Browser::Firefox.name().into(),
                publisher: Some("Vibe Work".into()), source: "Shipped with Vibe Work".into(), licence: None,
                runs: if !super::firefox_sharing::PAGE_SHARING_AVAILABLE {
                    "Adds a small program to Vibe Work's folder for your user and tells Firefox where it is."
                } else if self.shared.chrome_control.config.firefox_addon.is_some() {
                    "Adds a small program to Vibe Work's folder for your user and tells Firefox where it is. Then Firefox asks you to add the Vibe Work add-on."
                } else {
                    "Adds a small program to Vibe Work's folder for your user and tells Firefox where it is. Opens Firefox Add-ons to add Vibe Work."
                }
                .into(),
                version: None, expires_at: 0,
            }, self.shared.asks.now_ms().into(),
        )
    }

    pub(super) async fn firefox_setup(&self, offer_id: String) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        self.shared.install_offers.take(
            &offer_id,
            &OfferKey::new(KIND, setup::PLUGIN, "firefox"),
            self.shared.asks.now_ms().into(),
        )?;
        let state = chrome
            .dir()
            .ok_or_else(|| invalid(vw_vocabulary::texts::CHROME_NO_APP_FOLDER))?;
        let keys = self
            .shared
            .config
            .keys
            .as_ref()
            .ok_or_else(|| invalid(vw_vocabulary::texts::CHROME_NEEDS_KEYRING))?;
        if keys
            .chrome_owner()
            .await
            .map_err(|_| invalid(vw_vocabulary::texts::CHROME_OWNER_READ_FAILED))?
            .is_none()
        {
            keys.create_chrome_owner()
                .await
                .map_err(|_| invalid(vw_vocabulary::texts::CHROME_OWNER_STORE_FAILED))?;
        }
        setup::install_for(
            &chrome.config,
            state,
            &self.shared.instructions.own_home,
            NAME,
        )
        .map_err(|e| invalid(&e))?;
        chrome
            .set_browser_enabled(Browser::Firefox, true)
            .map_err(|e| invalid(&e))?;
        super::start_owner_for(&self.shared, Some(NAME)).await;
        super::switches::deliver(&self.shared).await?;
        if super::firefox_sharing::PAGE_SHARING_AVAILABLE {
            return self.firefox_addons();
        }
        super::owner::publish(&self.shared);
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
            hint: None,
        })
    }

    pub(super) fn firefox_addons(&self) -> Result<CommandResult, CoreError> {
        if !super::firefox_sharing::PAGE_SHARING_AVAILABLE {
            return Err(invalid(&super::firefox_sharing::page_sharing_unavailable()));
        }
        let config = &self.shared.chrome_control.config;
        // A current add-on already in Firefox (on or off) is managed in Add-ons; otherwise the bundled signed file
        // makes Firefox show its own install prompt.
        let sharing = super::firefox_sharing::view(&self.shared);
        let installed =
            sharing.extension_version.is_some() && sharing.state != super::AddonState::UpdateNeeded;
        let target = super::firefox_package::target(config, installed).map_err(|e| invalid(&e))?;
        let url = target.as_str();
        if let Some(launch) = &config.firefox_launcher {
            launch(url).map_err(|e| invalid(&e))?;
        } else {
            let executable = super::firefox_commands::executable().ok_or_else(|| {
                invalid("Firefox is missing. Install Mozilla Firefox, then try Connect again.")
            })?;
            vw_platform::process_command::hidden(&executable)
                .args(["--new-tab", url])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|_| {
                    invalid("Firefox could not open Add-ons. Open Firefox and try Connect again.")
                })?;
        }
        super::owner::publish(&self.shared);
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
            hint: None,
        })
    }

    pub(super) async fn firefox_remove(&self) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        let state = chrome
            .dir()
            .ok_or_else(|| invalid(vw_vocabulary::texts::CHROME_NO_APP_FOLDER))?;
        if !chrome.owner.is_running()
            && state.join("paired-profiles.json").is_file()
            && !chrome
                .owner
                .credential_missing
                .load(std::sync::atomic::Ordering::SeqCst)
            && let Err(reason) = super::owner::start_for_removal(&self.shared).await
            && !chrome
                .owner
                .credential_missing
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(invalid(&reason));
        }
        if chrome.owner.is_running() {
            let closed = chrome
                .owner
                .call("chrome_status", json!({"_firefoxWindow":"close"}))
                .await;
            if closed.value()["structuredContent"]["firefoxWindow"] != "Window closed" {
                return Err(invalid(
                    "The Firefox window could not close. Close its window, then try Remove again.",
                ));
            }
            if !chrome
                .owner
                .rpc("owner.browserUnpair", json!({"browser":"firefox"}))
                .await
                .is_some_and(|v| v["unpaired"] == true)
            {
                return Err(invalid(
                    "The Firefox connection could not be removed. Try Remove again.",
                ));
            }
        }
        chrome
            .set_browser_enabled(Browser::Firefox, false)
            .map_err(|e| invalid(&e))?;
        super::switches::stop_work(&self.shared, Browser::Firefox);
        chrome
            .owner
            .connected_profiles
            .remove_browser(Browser::Firefox);
        super::profiles::remove_browser(chrome, Browser::Firefox);
        chrome
            .chat_profiles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|_, profile| !Browser::Firefox.owns_profile(profile));
        chrome
            .work
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove_browser(Browser::Firefox);
        *chrome
            .owner
            .firefox_share_pairing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            (super::PairingState::Unknown, None);
        if setup::registered(&chrome.config, Some(state), NAME) {
            setup::remove_browser(&chrome.config, state, NAME, true).map_err(|e| invalid(&e))?;
        }
        match std::fs::remove_file(state.join("firefox-window")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(invalid(
                    "The Firefox connection could not be removed. Try Remove again.",
                ));
            }
        }
        *chrome
            .owner
            .firefox_pairing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            (super::PairingState::WindowClosed, None);
        super::switches::deliver(&self.shared).await?;
        super::owner::publish(&self.shared);
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
            hint: None,
        })
    }
}
