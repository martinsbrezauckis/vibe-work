//! Explicit Firefox add-on registration and removal. Uses the shared helper receipt and person-confirmed offers.
use super::{agent_browser::Browser, commands::invalid, setup};
use crate::install_offers::{OfferKey, ToolsInstallOffer};
use crate::{CommandResult, Core, CoreError};
use serde_json::json;
use vw_platform::sync::LockRecover;

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
            .map_err(|error| {
                invalid(&super::setup::owner_read_text(
                    super::agent_browser::Browser::Firefox,
                    &error,
                ))
            })?
            .is_none()
        {
            keys.create_chrome_owner().await.map_err(|error| {
                invalid(&super::setup::owner_store_text(
                    super::agent_browser::Browser::Firefox,
                    &error,
                ))
            })?;
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
        // A set-up Firefox gets its own host registration back first (a manifest the person or a cleanup removed,
        // or one that changed): the add-on added next must find the helper. Another setup's file stays refused.
        if let Some(state) = self.shared.chrome_control.dir()
            && setup::registered(config, Some(state), NAME)
        {
            setup::repair_registrations(config, state, Some(NAME)).map_err(|e| invalid(&e))?;
        }
        // A current add-on already in Firefox (on or off) is managed in Add-ons; otherwise the bundled signed file
        // makes Firefox show its own install prompt.
        let sharing = super::firefox_sharing::view(&self.shared);
        let installed =
            sharing.extension_version.is_some() && sharing.state != super::AddonState::UpdateNeeded;
        let executable = super::firefox_commands::executable();
        let target = super::firefox_package::target_for(config, installed, executable.as_deref())
            .map_err(|e| invalid(&e))?;
        let url = target.as_str();
        if let Some(launch) = &config.firefox_launcher {
            launch(url).map_err(|e| invalid(&e))?;
        } else {
            let executable = executable.ok_or_else(|| {
                invalid("Firefox is missing. Install Mozilla Firefox, then try Connect again.")
            })?;
            let (program, args) = connect_launch(&executable, url, cfg!(target_os = "macos"));
            vw_platform::process_command::hidden(&program)
                .args(args)
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
            return Err(invalid(
                &super::agent_browser::Browser::Firefox.text(&reason),
            ));
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
        let unsaved = super::profiles::remove_browser_saved(chrome, Browser::Firefox).is_err();
        chrome
            .chat_profiles
            .lock_or_recover()
            .retain(|_, profile| !Browser::Firefox.owns_profile(profile));
        chrome
            .work
            .lock_or_recover()
            .remove_browser(Browser::Firefox);
        *chrome.owner.firefox_share_pairing.lock_or_recover() =
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
        *chrome.owner.firefox_pairing.lock_or_recover() = (super::PairingState::WindowClosed, None);
        super::switches::deliver(&self.shared).await?;
        super::owner::publish(&self.shared);
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
            hint: unsaved.then(|| Browser::Firefox.words(super::profiles::REMOVAL_NOT_SAVED)),
        })
    }
}

/// How Connect hands Firefox its page. On macOS the URL goes through Launch Services (`open -a Firefox.app`),
/// which gives it to the person's running Firefox or starts Firefox normally with its own default profile;
/// starting the `firefox` binary directly beside a running Firefox cannot load that locked profile and shows
/// Firefox's "Profile Missing" window. Elsewhere the binary's `--new-tab` reaches the running Firefox itself.
fn connect_launch(
    executable: &std::path::Path,
    url: &str,
    macos: bool,
) -> (std::path::PathBuf, Vec<std::ffi::OsString>) {
    let bundle = executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"));
    match bundle.filter(|_| macos) {
        Some(app) => (
            "/usr/bin/open".into(),
            vec!["-a".into(), app.into(), url.into()],
        ),
        None => (executable.into(), vec!["--new-tab".into(), url.into()]),
    }
}

#[cfg(test)]
mod launch_tests {
    // PREDICT (before the fix): Connect on macOS ran `…/Firefox.app/Contents/MacOS/firefox --new-tab <url>`
    // directly, the launch Firefox answers with "Profile Missing" when its default profile is held by the running
    // Firefox. After: macOS opens the URL with `/usr/bin/open -a …/Firefox.app <url>`; other systems keep --new-tab.
    use super::connect_launch;
    use std::path::Path;
    #[test]
    fn macos_connect_opens_the_page_through_launch_services() {
        let url =
            "file:///Applications/Vibe%20Work.app/Contents/Resources/firefox-addon/vibe-work.xpi";
        for app in [
            "/Applications/Firefox.app",
            "/Users/a/Applications/Firefox.app",
        ] {
            let exe = Path::new(app).join("Contents/MacOS/firefox");
            let (program, args) = connect_launch(&exe, url, true);
            assert_eq!(program, Path::new("/usr/bin/open"));
            assert_eq!(args, ["-a", app, url].map(std::ffi::OsString::from));
            assert!(
                !args
                    .iter()
                    .any(|a| a == "--new-tab" || a == "-profile" || a == "-P")
            );
        }
    }
    #[test]
    fn other_systems_hand_the_page_to_the_running_firefox() {
        for exe in [
            "/usr/bin/firefox",
            r"C:\Program Files\Mozilla Firefox\firefox.exe",
        ] {
            let (program, args) = connect_launch(Path::new(exe), "about:addons", false);
            assert_eq!(program, Path::new(exe));
            assert_eq!(
                args,
                ["--new-tab", "about:addons"].map(std::ffi::OsString::from)
            );
        }
        // A Firefox outside an app bundle on macOS (none ships that way) keeps the direct launch.
        let (program, _) = connect_launch(Path::new("/opt/firefox/firefox"), "about:addons", true);
        assert_eq!(program, Path::new("/opt/firefox/firefox"));
    }
}
