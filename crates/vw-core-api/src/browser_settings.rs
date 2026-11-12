//! Persisted composer visibility and built-in availability. Settings changes never alter profile pairings.
//! Shared core state owns this model; browser admission and composer views read its confirmed values.
use crate::{BrowserRequest, BrowserResult, CoreConfig, CoreError, ErrorCode, core::Shared};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Mutex, PoisonError},
};

/// Browser represented in the composer's tab menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ComposerBrowser {
    /// Vibe Work's built-in browser.
    Builtin,
    /// The person's Chrome.
    Chrome,
    /// The person's Edge.
    Edge,
    /// The person's Safari.
    Safari,
    /// The person's Firefox.
    Firefox,
}

/// Person-owned presentation and built-in availability; external availability stays in each browser's switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserPreferences {
    /// Whether the agent can use the built-in browser.
    pub builtin_enabled: bool,
    /// Whether this app run has a built-in browser engine.
    #[serde(default)]
    pub builtin_available: bool,
    /// Browsers shown in the composer. Hiding one does not revoke a pairing or change an access mode.
    pub shown: Vec<ComposerBrowser>,
}
impl Default for BrowserPreferences {
    fn default() -> Self {
        Self {
            builtin_enabled: true,
            builtin_available: false,
            shown: vec![
                ComposerBrowser::Builtin,
                ComposerBrowser::Chrome,
                ComposerBrowser::Edge,
                ComposerBrowser::Safari,
                ComposerBrowser::Firefox,
            ],
        }
    }
}
pub(crate) struct Settings {
    path: Option<PathBuf>,
    current: Mutex<BrowserPreferences>,
}
impl Settings {
    pub(crate) fn new(config: &CoreConfig) -> Self {
        let path = config
            .preview
            .as_ref()
            .map(|preview| {
                preview
                    .config
                    .storage_root
                    .join("browser-presentation.json")
            })
            .or_else(|| {
                config
                    .secret_scan_data
                    .as_ref()
                    .map(|root| root.join("browser-presentation.json"))
            });
        let off = || BrowserPreferences {
            builtin_enabled: false,
            builtin_available: false,
            shown: Vec::new(),
        };
        let current = match path.as_ref().map(std::fs::read) {
            Some(Ok(bytes)) => serde_json::from_slice(&bytes).unwrap_or_else(|_| off()),
            Some(Err(error)) if error.kind() != std::io::ErrorKind::NotFound => off(),
            _ => BrowserPreferences::default(),
        };
        Self {
            path,
            current: Mutex::new(current),
        }
    }
    pub(crate) fn view(&self, available: bool) -> BrowserPreferences {
        let mut view = self
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        view.builtin_available = available;
        view
    }
    pub(crate) fn enabled(&self) -> bool {
        self.current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .builtin_enabled
    }
    fn update(&self, change: impl FnOnce(&mut BrowserPreferences)) -> Result<(), CoreError> {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = current.clone();
        change(&mut next);
        if let Some(path) = &self.path {
            let save = || -> Result<(), String> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let data = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
                vw_platform::replace_private(path, &data).map_err(|e| e.to_string())
            };
            save().map_err(|_| {
                CoreError::new(
                    ErrorCode::Internal,
                    "Could not save the browser setting. Try again.",
                )
            })?;
        }
        *current = next;
        Ok(())
    }
}
pub(crate) fn check(shared: &Shared) -> Result<(), String> {
    if shared.browser_settings.enabled() {
        Ok(())
    } else {
        Err("Vibe Work browser is off. Ask the user to turn it on in Settings > Browser > Vibe Work browser before using browser tools.".into())
    }
}
pub(crate) fn run(
    shared: &Shared,
    request: &BrowserRequest,
) -> Option<Result<BrowserResult, CoreError>> {
    let changed = match request {
        BrowserRequest::SetComposerBrowser { browser, shown } => {
            shared.browser_settings.update(|prefs| {
                prefs.shown.retain(|choice| choice != browser);
                if *shown {
                    prefs.shown.push(*browser);
                }
            })
        }
        BrowserRequest::SetBuiltinEnabled { enabled } => shared
            .browser_settings
            .update(|prefs| prefs.builtin_enabled = *enabled),
        BrowserRequest::Preferences {} => Ok(()),
        _ => return None,
    };
    Some(changed.map(|()| {
        if matches!(
            request,
            BrowserRequest::SetBuiltinEnabled { enabled: false }
        ) {
            let owners = shared
                .store
                .views()
                .into_iter()
                .map(|view| view.session_id)
                .chain(std::iter::once(shared.new_chat.owner_id()));
            for owner in owners {
                crate::browser::stop_for_chat(shared, &owner);
            }
        }
        shared.emit(crate::CoreEvent::ChromeControl {
            view: crate::chrome_control::commands::view(shared),
        });
        BrowserResult::Preferences {
            preferences: shared.browser_settings.view(shared.preview.is_some()),
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visibility_persists_without_switching_off_availability() {
        let path =
            std::env::temp_dir().join(format!("browser-presentation-{}", std::process::id()));
        let config = CoreConfig {
            secret_scan_data: Some(path.clone()),
            ..CoreConfig::default()
        };
        let state = Settings::new(&config);
        state
            .update(|prefs| {
                prefs
                    .shown
                    .retain(|browser| *browser != ComposerBrowser::Chrome);
            })
            .unwrap();
        let restored = Settings::new(&config);
        assert!(restored.enabled());
        assert!(!restored.view(true).shown.contains(&ComposerBrowser::Chrome));
        restored
            .update(|prefs| prefs.builtin_enabled = false)
            .unwrap();
        assert!(!Settings::new(&config).enabled());
        std::fs::remove_dir_all(path).unwrap();
    }
}
