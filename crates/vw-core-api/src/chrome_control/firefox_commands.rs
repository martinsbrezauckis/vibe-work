//! Optional app-owned Firefox window commands. Uses the checked helper and existing owner key; no add-on setup.
use super::{
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
        setup.pairing = match pairing.0.as_str() {
            "Window open" | "Window closed" | "Not installed" => pairing.0.clone(),
            _ if installed() => "Window closed".into(),
            _ => "Not installed".into(),
        };
        setup.state = if setup.pairing == "Window open" {
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
fn installed() -> bool {
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
    paths.iter().any(|p| p.is_file())
}
impl Core {
    pub(super) async fn firefox_window(&self, open: bool) -> Result<CommandResult, CoreError> {
        if !cfg!(feature = "your-firefox") {
            return Err(invalid(ABSENT));
        }
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
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
            .filter(|s| matches!(*s, "Window open" | "Window closed" | "Not installed"));
        if let Some(state) = state {
            *chrome
                .owner
                .firefox_pairing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = (state.into(), None);
        } else {
            return Err(invalid(data["error"]["message"].as_str().unwrap_or(
                "The Firefox connection is unavailable. Try opening the window again.",
            )));
        }
        let view = view(&self.shared);
        self.shared
            .emit(crate::CoreEvent::ChromeControl { view: view.clone() });
        Ok(CommandResult::ChromeControl { view, offer: None })
    }
}
