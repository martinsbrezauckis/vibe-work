//! Owned Firefox window commands. Uses the checked helper and existing owner key; sharing setup is separate.
use super::{
    PairingState,
    commands::{invalid, view},
    setup,
};
use crate::{CommandResult, Core, CoreError};
use serde_json::json;

pub(super) const ABSENT: &str = vw_vocabulary::texts::FIREFOX_UNAVAILABLE;

pub(super) fn setup_view(shared: &crate::core::Shared) -> Option<Box<setup::ChromeSetupView>> {
    cfg!(feature = "your-firefox").then(|| {
        let chrome = &shared.chrome_control;
        let mut setup = setup::view_for(
            &chrome.config,
            chrome.dir(),
            &shared.instructions.own_home,
            "Mozilla Firefox",
        );
        let pairing = chrome
            .owner
            .firefox_pairing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        setup.pairing_state = match pairing.0 {
            PairingState::WindowOpen
            | PairingState::WindowClosed
            | PairingState::NotInstalled
            | PairingState::Connecting
            | PairingState::BrowserOutdated
            | PairingState::Blocked => pairing.0,
            _ if executable().is_some() => PairingState::WindowClosed,
            _ => PairingState::NotInstalled,
        };
        let facts = state_view(setup.pairing_state, chrome.firefox_enabled(), setup.package);
        setup.pairing.clone_from(&facts.label);
        setup.addon = Some(facts);
        setup.state = if setup.pairing_state == PairingState::WindowOpen {
            setup::SetupState::ExtensionAdded
        } else {
            setup::SetupState::NotSetUp
        };
        setup.store_url.clear();
        setup.store_review = false;
        setup.fingerprint = None;
        setup.states = None;
        Box::new(setup)
    })
}
pub(super) fn executable() -> Option<std::path::PathBuf> {
    let mut paths = Vec::new();
    if cfg!(target_os = "macos") {
        paths.push(std::path::PathBuf::from(
            "/Applications/Firefox.app/Contents/MacOS/firefox",
        ));
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(
                std::path::PathBuf::from(home)
                    .join("Applications/Firefox.app/Contents/MacOS/firefox"),
            );
        }
    }
    if cfg!(windows) {
        for key in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(key) {
                paths.push(std::path::PathBuf::from(root).join("Mozilla Firefox/firefox.exe"));
            }
        }
    }
    if let Some(value) = std::env::var_os("PATH") {
        paths.extend(
            std::env::split_paths(&value)
                .filter(|p| p.is_absolute())
                .map(|p| {
                    p.join(if cfg!(windows) {
                        "firefox.exe"
                    } else {
                        "firefox"
                    })
                }),
        );
    }
    paths.into_iter().find(|p| p.is_file())
}
impl Core {
    pub(super) async fn firefox_window(&self, open: bool) -> Result<CommandResult, CoreError> {
        if !cfg!(feature = "your-firefox") {
            return Err(invalid(ABSENT));
        }
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        if open && !chrome.firefox_enabled() {
            return Err(invalid(&super::agent_browser::Browser::Firefox.words(
                "{yourBrowser} is off. Turn it on in Settings > Browser > {yourBrowser}.",
            )));
        }
        if open {
            setup::packaged_helper(&chrome.config).map_err(|e| invalid(&e))?;
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
            std::fs::create_dir_all(state)
                .map_err(|_| invalid("The Firefox connection could not be saved."))?;
            std::fs::write(state.join("firefox-window"), b"enabled\n")
                .map_err(|_| invalid("The Firefox connection could not be saved."))?;
            super::start_owner(&self.shared).await;
        }
        if open {
            *chrome
                .owner
                .firefox_pairing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                (PairingState::Connecting, None);
            self.shared.emit(crate::CoreEvent::ChromeControl {
                view: view(&self.shared),
            });
        }
        let answer = chrome
            .owner
            .call(
                "chrome_status",
                json!({"_firefoxWindow":if open {"open"} else {"close"}}),
            )
            .await;
        let data = &answer.value()["structuredContent"];
        let state = data["firefoxWindow"]
            .as_str()
            .and_then(PairingState::firefox_window);
        if let Some(state) = state {
            *chrome
                .owner
                .firefox_pairing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = (state, None);
        } else {
            let state = match data["error"]["code"].as_str() {
                Some("BROWSER_NOT_INSTALLED") => PairingState::NotInstalled,
                Some("BROWSER_OUTDATED") => PairingState::BrowserOutdated,
                _ => PairingState::WindowClosed,
            };
            *chrome
                .owner
                .firefox_pairing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = (state, None);
            self.shared.emit(crate::CoreEvent::ChromeControl {
                view: view(&self.shared),
            });
            let fallback = format!(
                "{} Ask the user to open its window in Vibe Work Settings, then check chrome_status.",
                vw_vocabulary::texts::FIREFOX_NOT_CONNECTED
            );
            return Err(invalid(
                data["error"]["message"].as_str().unwrap_or(&fallback),
            ));
        }
        let view = view(&self.shared);
        self.shared
            .emit(crate::CoreEvent::ChromeControl { view: view.clone() });
        Ok(CommandResult::ChromeControl { view, offer: None })
    }
}

/// Firefox's card and agent facts share one diagnosis and name the actual window controls.
pub(super) fn addon(shared: &crate::core::Shared) -> super::AddonView {
    setup_view(shared)
        .and_then(|v| v.addon)
        .unwrap_or_else(|| state_view(PairingState::Unknown, false, false))
}
fn state_view(state: PairingState, enabled: bool, package: bool) -> super::AddonView {
    use super::AddonState as A;
    let (state, label, instruction) = if (!package && state != PairingState::WindowOpen)
        || !cfg!(feature = "your-firefox")
    {
        (
            A::Unavailable,
            "Unavailable",
            super::browser_text::UNAVAILABLE,
        )
    } else if state == PairingState::NotInstalled {
        (
            A::ChromeMissing,
            "Not installed",
            "Firefox is missing. Ask the user to install Firefox, then choose Open window in Settings > Browser > {yourBrowser}.",
        )
    } else if !enabled {
        (A::SwitchedOff, "Off", super::browser_text::SWITCHED_OFF)
    } else {
        match state {
            PairingState::WindowOpen => (
                A::Connected,
                "Window open",
                "The owned Firefox window is ready. Use chrome_session with profileId firefox:agent to open this chat's session; use chrome_status for its supported actions.",
            ),
            PairingState::Connecting => (
                A::Connecting,
                "Starting",
                "The owned Firefox window is starting. Wait, then check chrome_status for its connection state.",
            ),
            PairingState::BrowserOutdated => (
                A::UpdateNeeded,
                "Update Firefox",
                "Firefox is older than the supported version. Ask the user to update Firefox to version 159 or later, then choose Open window under Settings > Browser > {yourBrowser}.",
            ),
            PairingState::Blocked => (
                A::Unavailable,
                "Blocked by your organization",
                "The user's organization blocks this Firefox connection. Ask the user to contact their administrator.",
            ),
            _ => (
                A::ChromeClosed,
                "Window closed",
                "The owned Firefox window is closed. Ask the user to choose Open window under Settings > Browser > {yourBrowser}, then open a new chat browser session; the old session cannot resume after a restart.",
            ),
        }
    };
    super::AddonView {
        state,
        label: label.into(),
        instruction: super::agent_browser::Browser::Firefox.words(instruction),
        action: None,
        extension_version: None,
        step: None,
    }
}

#[cfg(all(test, feature = "your-firefox"))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn firefox_facts_use_its_window_controls_and_connection_state() {
        let core = crate::offline_core::start_with("firefox-window-facts", |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
        })
        .await;
        super::super::test_support::answering(&core.shared, |_, _| json!({"structuredContent":{}}));
        for (state, label, instruction) in [
            (PairingState::WindowClosed, "Window closed", "Open window"),
            (PairingState::WindowOpen, "Window open", "chrome_session"),
            (PairingState::Connecting, "Starting", "Wait"),
            (
                PairingState::BrowserOutdated,
                "Update Firefox",
                "Open window",
            ),
            (
                PairingState::NotInstalled,
                "Not installed",
                "Ask the user to install Firefox",
            ),
        ] {
            *core
                .shared
                .chrome_control
                .owner
                .firefox_pairing
                .lock()
                .unwrap() = (state, None);
            let facts = super::super::installation::addons(&core.shared);
            let firefox = facts
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["id"] == "firefox")
                .expect("Firefox facts are required when its module is enabled");
            assert_eq!(firefox["label"], label);
            assert!(
                firefox["instruction"]
                    .as_str()
                    .unwrap()
                    .contains(instruction),
                "{firefox}"
            );
            assert!(
                !firefox["instruction"]
                    .as_str()
                    .unwrap()
                    .contains("Connect Firefox")
            );
            if state == PairingState::NotInstalled {
                assert_eq!(
                    firefox["instruction"],
                    format!(
                        "Firefox is missing. Ask the user to install Firefox, then choose Open window in Settings > Browser > {}.",
                        vw_vocabulary::texts::FIREFOX_NAME
                    )
                );
            }
        }
        core.shutdown().await;
    }
}
