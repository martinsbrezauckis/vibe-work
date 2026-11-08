//! Optional Safari setup and agent facts. Owns its opt-in marker; the shared helper retains modes and pairing.
//! Local and public websites use Safari site grants; browser mode, ownership and Stop remain shared.
use super::{AddonState, AddonView, agent_browser::Browser};
use crate::{CommandResult, Core, CoreError, core::Shared};
use serde_json::{Value, json};

const PAGE_INSTRUCTION: &str = "Safari uses website permission for public and local URLs. Use tabs in this chat window or handed windows and supported actions. Tabs opened in handed windows stay open until closed; the close tools can close them when the task no longer needs them. Input is synthetic and untrusted; read the page to confirm its result.";
fn absent() -> String {
    Browser::Safari.words("{yourBrowser} is unavailable in this build.")
}

pub(super) enum ConnectionError {
    Container,
    Relay,
}

pub(super) fn connection(shared: &Shared, state: &str) {
    if !cfg!(feature = "your-safari") {
        return;
    }
    let error = match state {
        "containerUnavailable" => Some(ConnectionError::Container),
        "unavailable" => Some(ConnectionError::Relay),
        "ready" | "preparing" => None,
        _ => return,
    };
    *shared
        .chrome_control
        .owner
        .safari_error
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = error;
    super::owner::publish(shared);
}

/// Safari's optional Browser Settings card; omitted completely in the default build.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SafariView {
    /// The person's independent, initially off switch.
    pub enabled: bool,
    /// Whether this app contains the Safari extension, setup utility and native helper.
    pub package: bool,
    /// Same short connection states as the Chrome card.
    pub addon: AddonView,
    /// The helper’s own pairing identity, only while a live Safari pairing event supplies it.
    pub fingerprint: Option<String>,
}

pub(crate) fn enabled(shared: &Shared) -> bool {
    cfg!(feature = "your-safari")
        && shared.chrome_control.dir().is_some_and(|dir| {
            std::fs::read(dir.join("safari-enabled.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<bool>(&bytes).ok())
                == Some(true)
        })
}

fn version(shared: &Shared) -> Option<String> {
    shared
        .chrome_control
        .owner
        .connected_profiles
        .snapshot()
        .into_iter()
        .filter(|(id, _)| id.starts_with("safari:"))
        .filter_map(|(_, version)| version)
        .min_by(|a, b| {
            vw_vibe_install::support::compare_versions(a, b).unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn diagnose(package: bool, on: bool, version: Option<String>) -> AddonView {
    let package = package || version.is_some();
    let state = if !package {
        AddonState::Unavailable
    } else if !on {
        AddonState::SwitchedOff
    } else if version.as_deref().is_some_and(|version| {
        !matches!(
            vw_vibe_install::support::compare_versions(version, "0.1.8"),
            Some(std::cmp::Ordering::Equal | std::cmp::Ordering::Greater)
        )
    }) {
        AddonState::UpdateNeeded
    } else if version.is_some() {
        AddonState::Connected
    } else {
        AddonState::NotPaired
    };
    let mut addon = super::addon::view_for(state, "Unavailable", Browser::Safari);
    addon.extension_version = version;
    if state == AddonState::Unavailable {
        addon.instruction = format!(
            "{} Ask the person to install a build with {}.",
            absent(),
            vw_vocabulary::texts::SAFARI_NAME
        );
    }
    if state == AddonState::Connected {
        addon.instruction.push(' ');
        addon.instruction.push_str(PAGE_INSTRUCTION);
    }
    if state == AddonState::NotPaired {
        addon.instruction = Browser::Safari.words("Safari has no confirmed paired extension connection. Ask the person to click Open pairing page, enable {yourBrowser} in Safari Settings if asked, then pair after comparing its identity with Vibe Work.");
    }
    if state == AddonState::UpdateNeeded {
        addon.instruction = "The connected Safari extension is older than this Vibe Work needs. Ask the person to update Vibe Work, then reconnect Safari.".into();
    }
    addon
}

fn removed(shared: &Shared) -> bool {
    shared
        .chrome_control
        .dir()
        .is_some_and(|dir| dir.join("safari-removed").is_file())
}

pub(super) fn view(shared: &Shared) -> Option<SafariView> {
    cfg!(feature = "your-safari").then(|| {
        let enabled = enabled(shared);
        let version = version(shared);
        let package = version.is_some() || (cfg!(target_os = "macos")
            && shared.chrome_control.config.available() && super::safari_package::installed());
        let mut addon = diagnose(package, enabled, version);
        let pairing = shared.chrome_control.owner.safari_pairing.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if removed(shared) {
            addon = super::addon::view_for(AddonState::NotSetUp, "", Browser::Safari);
        } else if enabled && pairing.0 == "mismatch" {
            addon = super::addon::view_for(AddonState::OtherOwner, "", Browser::Safari);
        } else if enabled && let Some(error) = &*shared.chrome_control.owner.safari_error.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
            addon = super::addon::view_for(AddonState::HelperSetupRequired, "", Browser::Safari);
            addon.instruction = match error {
                ConnectionError::Container => "Safari's shared container could not be prepared. Ask the person to click Set up again; if it still fails, reinstall Vibe Work.",
                ConnectionError::Relay => "Safari's helper socket could not be created. Ask the person to click Set up again; if it still fails, restart Vibe Work.",
            }.into();
        }
        let fingerprint = matches!(addon.state, AddonState::NotPaired | AddonState::OtherOwner).then(|| pairing.1.clone()).flatten();
        SafariView { enabled, package, addon, fingerprint }
    })
}

pub(super) fn observe(shared: &Shared, params: &Value) {
    if !cfg!(feature = "your-safari")
        || !params["profileId"]
            .as_str()
            .is_some_and(|id| id.starts_with("safari:"))
    {
        return;
    }
    let Some(state) = params["state"]
        .as_str()
        .filter(|s| matches!(*s, "waiting" | "paired" | "mismatch"))
    else {
        return;
    };
    let fingerprint = params["fingerprint"]
        .as_str()
        .filter(|v| {
            v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()) && state != "mismatch"
        })
        .map(str::to_owned);
    *shared
        .chrome_control
        .owner
        .safari_pairing
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = (state.into(), fingerprint);
    // A pairing state reaches the helper only from a connected extension (`safari_package::open` waits on this).
    shared
        .chrome_control
        .owner
        .safari_seen
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    super::owner::publish(shared);
}

pub(super) fn agent_fact(shared: &Shared) -> Option<Value> {
    // Match the shared add-on fact shape. The connected instruction states the page contract once;
    // live helper profiles supply the action list and synthetic/local-site/owned-tab capability markers.
    view(shared).map(|view| {
        json!({"id":"safari","name":vw_vocabulary::texts::SAFARI_NAME,"state":view.addon.label,
        "stateCode":view.addon.state,"label":view.addon.label,"instruction":view.addon.instruction,
        "extensionVersion":view.addon.extension_version})
    })
}

/// Additional browser facts share repeated profile metadata only when the optional build exceeds its budget.
/// The raw helper state remains authoritative; this changes only the model-facing status representation.
pub(super) fn compact_overview(body: &mut Value) {
    if !cfg!(feature = "your-safari") || super::output::fits(body, 6000) {
        return;
    }
    let status = &mut body["untrustedPageContent"];
    let Some(profiles) = status["profiles"]
        .as_array_mut()
        .filter(|rows| rows.len() > 1)
    else {
        return;
    };
    let mut defaults = serde_json::Map::new();
    for key in [
        "browser",
        "connected",
        "browserVersion",
        "extensionVersion",
        "route",
        "features",
        "extensionCapabilities",
    ] {
        if let Some(value) = profiles[0].get(key).cloned()
            && profiles
                .iter()
                .all(|profile| profile.get(key) == Some(&value))
        {
            defaults.insert(key.into(), value);
            for profile in profiles.iter_mut() {
                profile.as_object_mut().map(|fields| fields.remove(key));
            }
        }
    }
    if defaults.is_empty() {
        return;
    }
    status["profileDefaults"] = Value::Object(defaults);
    if let Some(instruction) = status["capabilities"]["availability"].as_str() {
        status["capabilities"]["availability"] = json!(instruction.replace(
            "Profile feature lists define support.",
            "Profiles inherit profileDefaults."
        ));
    }
    // A missing store verdict is already unknown; do not repeat an explicit null in this crowded overview.
    if status
        .get("chromeExtensionStore")
        .is_some_and(Value::is_null)
    {
        status
            .as_object_mut()
            .map(|fields| fields.remove("chromeExtensionStore"));
    }
}

/// Intercepts Safari selection before Chrome reconnect/retargeting logic. Never silently switches browsers.
pub(super) fn selected(shared: &Shared, session: &str, tool: &str, args: &Value) -> bool {
    if !cfg!(feature = "your-safari") {
        return false;
    }
    let safari = |id: &str| id == "safari" || id.starts_with("safari:");
    if tool == "chrome_status"
        && let Some(id) = args["profileId"].as_str()
    {
        return safari(id);
    }
    // Open requests select a profile; existing session calls use only their retained binding.
    // An extra profileId on execute/get/stop cannot select another browser's switch or policy.
    if let Some(id) = super::receipt::profile(shared, tool, args) {
        return safari(&id);
    }
    shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(session)
        .is_some_and(|id| safari(id))
}

pub(super) async fn disabled(shared: &std::sync::Arc<Shared>) {
    let chrome = &shared.chrome_control;
    chrome
        .safari_generation
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    super::switches::stop_work(shared, super::agent_browser::Browser::Safari);
    if !shared.chrome_control.any_enabled() {
        super::stop_owner(shared).await;
    }
}

/// The explicit repair action retries native relay setup; ordinary reads and healthy connections never restart it.
pub(super) async fn repair_connection(shared: &std::sync::Arc<Shared>) {
    let broken = shared
        .chrome_control
        .owner
        .safari_error
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some();
    if enabled(shared) && broken {
        super::stop_owner(shared).await;
        super::start_owner(shared).await;
    }
}

impl Core {
    pub(super) async fn safari_enabled(&self, on: bool) -> Result<CommandResult, CoreError> {
        if !cfg!(all(feature = "your-safari", target_os = "macos")) {
            return Err(super::commands::invalid(&absent()));
        }
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        self.safari_enabled_held(on).await
    }

    async fn safari_enabled_held(&self, on: bool) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        let dir = chrome
            .dir()
            .ok_or_else(|| super::commands::invalid("Safari settings are unavailable."))?;
        if on {
            super::setup::packaged_helper(&chrome.config)
                .map_err(|e| super::commands::invalid(&e))?;
            let keys = self.shared.config.keys.as_ref().ok_or_else(|| {
                super::commands::invalid("Safari pairing needs the system keyring.")
            })?;
            if keys
                .chrome_owner()
                .await
                .map_err(|_| super::commands::invalid("The pairing key could not be read."))?
                .is_none()
            {
                keys.create_chrome_owner()
                    .await
                    .map_err(|_| super::commands::invalid("The pairing key could not be saved."))?;
            }
        }
        if on {
            #[cfg(target_os = "macos")]
            super::safari_package::registration(false)
                .await
                .map_err(|reason| super::commands::invalid(&reason))?;
            if dir.join("safari-removed").exists() {
                std::fs::remove_file(dir.join("safari-removed"))
                    .map_err(|_| super::commands::invalid("Safari setup could not be saved."))?;
            }
        }
        {
            // Startup snapshots use this same lock: the saved flag and revision must describe one change.
            let _switches = chrome
                .switches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let failed = || super::commands::invalid("The Safari setting could not be saved.");
            std::fs::create_dir_all(dir).map_err(|_| failed())?;
            vw_platform::replace_private(
                &dir.join("safari-enabled.json"),
                if on { b"true" } else { b"false" },
            )
            .map_err(|_| failed())?;
            chrome
                .switch_generation
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        let delivered = super::switches::deliver(&self.shared).await;
        if on {
            super::start_owner(&self.shared).await;
        } else {
            disabled(&self.shared).await;
        }
        super::owner::publish(&self.shared);
        delivered?;
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
        })
    }

    pub(super) async fn safari_remove(&self) -> Result<CommandResult, CoreError> {
        if !cfg!(all(feature = "your-safari", target_os = "macos")) {
            return Err(super::commands::invalid(&absent()));
        }
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        self.safari_enabled_held(false).await?;
        #[cfg(target_os = "macos")]
        super::safari_package::registration(true)
            .await
            .map_err(|reason| super::commands::invalid(&reason))?;
        let dir = chrome
            .dir()
            .ok_or_else(|| super::commands::invalid("Safari settings are unavailable."))?;
        vw_platform::replace_private(&dir.join("safari-removed"), b"true")
            .map_err(|_| super::commands::invalid("Safari removal could not be saved."))?;
        *chrome
            .owner
            .safari_pairing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = (String::new(), None);
        super::owner::publish(&self.shared);
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
        })
    }

    pub(super) async fn safari_setup(&self) -> Result<CommandResult, CoreError> {
        if !cfg!(all(feature = "your-safari", target_os = "macos")) {
            return Err(super::commands::invalid(&absent()));
        }
        if !super::safari_package::installed() {
            return Err(super::commands::invalid(
                &Browser::Safari.words("{yourBrowser} is not installed."),
            ));
        }
        let _held = self.shared.chrome_control.lifecycle.lock().await;
        repair_connection(&self.shared).await;
        #[cfg(target_os = "macos")]
        super::safari_package::open(&self.shared)
            .map_err(|reason| super::commands::invalid(&reason))?;
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "your-safari")]
    #[tokio::test]
    async fn safari_disable_saves_and_delivers_its_switch_without_stopping_other_browsers() {
        let core = crate::offline_core::start_with("safari-switch-delivery", |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
        })
        .await;
        let chrome = &core.shared.chrome_control;
        let dir = chrome.dir().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
        let calls = chrome.owner.answering(|_, _| json!({"applied":true}));
        core.safari_enabled_held(false).await.unwrap();
        assert_eq!(
            std::fs::read(dir.join("safari-enabled.json")).unwrap(),
            b"false"
        );
        assert!(!enabled(&core.shared));
        assert!(chrome.enabled() && chrome.edge_enabled());
        assert!(chrome.owner.is_running());
        assert!(calls.lock().unwrap().iter().any(|(method, params)| {
            method == "owner.browserAvailability"
                && params["safari"] == false
                && params["chrome"] == true
                && params["generation"] == 1
        }));
        core.shutdown().await;
    }

    #[cfg(feature = "your-safari")]
    #[tokio::test]
    async fn safari_pairing_start_does_not_require_chromes_switch() {
        let core = crate::offline_core::start_with("safari-independent-switch", |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
        })
        .await;
        let chrome = &core.shared.chrome_control;
        chrome.set_enabled(false).unwrap();
        let dir = chrome.dir().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
        // No test keyring: reaching that check proves independent start admission without launching a helper.
        assert!(super::super::owner::start(&core.shared).await.is_err());
        assert!(!chrome.enabled());
        core.shutdown().await;
    }

    #[test]
    fn safari_connection_labels_and_actions_match_the_shared_browser_model() {
        for (on, version) in [
            (false, None),
            (true, None),
            (true, Some("0.1.5")),
            (true, Some("0.1.8")),
        ] {
            let actual = diagnose(true, on, version.map(str::to_owned));
            let expected = super::super::addon::view_for(
                actual.state,
                "",
                super::super::agent_browser::Browser::Safari,
            );
            assert_eq!(actual.label, expected.label);
            assert_eq!(actual.action, expected.action);
        }
    }

    #[test]
    fn safari_has_a_separate_remove_command() {
        let request = serde_json::from_value::<super::super::commands::ChromeControlRequest>(
            json!({"op":"removeSafari"}),
        );
        assert!(
            request.is_ok(),
            "Safari removal cannot remove Chrome or Edge: {request:?}"
        );
    }

    #[test]
    fn missing_off_unpaired_outdated_and_paired_remain_distinct() {
        assert_eq!(diagnose(false, true, None).state, AddonState::Unavailable);
        assert_eq!(diagnose(true, false, None).state, AddonState::SwitchedOff);
        assert_eq!(diagnose(true, true, None).state, AddonState::NotPaired);
        assert_eq!(
            diagnose(true, true, Some("0.1.5".into())).state,
            AddonState::UpdateNeeded
        );
        let paired = diagnose(true, true, Some("0.1.8".into()));
        assert_eq!(paired.state, AddonState::Connected);
        assert!(paired.instruction.contains("website permission"));
        assert!(paired.instruction.contains("read the page"));
    }
}
