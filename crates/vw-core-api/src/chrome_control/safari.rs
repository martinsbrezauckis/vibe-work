//! Optional Safari setup and agent facts. Owns its opt-in marker; the shared helper retains modes and pairing.
//! Local and public websites use Safari site grants; browser mode, ownership and Stop remain shared.
use super::{AddonState, AddonView, agent_browser::Browser};
use crate::{CommandResult, Core, CoreError, core::Shared};
use serde_json::{Value, json};

const PAGE_INSTRUCTION: &str = "Safari needs website permission for the site and System Settings > Privacy & Security > Accessibility for native input. Use this chat's owned or handed tabs. Native clicks, keys and drags bring Safari forward and move the real cursor; form setters are untrusted. To confirm the result, read the page.";
fn absent() -> String {
    super::browser_text::unavailable(Browser::Safari, "")
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
    /// Other registered installations containing the Safari extension; never changed by this app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub other_app_copies: Option<Vec<String>>,
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
            vw_vibe_install::support::compare_versions(version, crate::version()),
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
    if state == AddonState::Connected {
        addon.instruction.push(' ');
        addon.instruction.push_str(PAGE_INSTRUCTION);
    }
    if state == AddonState::NotPaired {
        addon.instruction = Browser::Safari.words("Safari has no confirmed paired extension connection. Ask the user to click Open pairing page, enable {yourBrowser} in Safari Settings if asked, then pair after comparing its identity with Vibe Work.");
    }
    if state == AddonState::UpdateNeeded {
        addon.instruction = "The connected Safari extension is older than this Vibe Work needs. Ask the user to update Vibe Work, then reconnect Safari.".into();
    }
    addon
}

/// Shown after the person approved a Safari pairing until the extension connects: Safari may not run the extension's
/// page again until its toolbar button is clicked, so the card says that step instead of looking unpaired.
pub(super) const FINISH_STEP: &str = "Click {yourBrowser} in Safari's toolbar to finish.";

pub(super) fn finishing(shared: &Shared, addon: &mut AddonView, enabled: bool) {
    let approved = &shared.chrome_control.owner.safari_approved;
    if addon.state == AddonState::Connected || !enabled {
        approved.store(false, std::sync::atomic::Ordering::SeqCst);
    } else if addon.state == AddonState::NotPaired
        && approved.load(std::sync::atomic::Ordering::SeqCst)
    {
        *addon = super::addon::view_for(AddonState::Connecting, "", Browser::Safari);
        addon.label = "Finish in Safari".into();
        addon.step = Some(Browser::Safari.words(FINISH_STEP));
        addon.instruction = Browser::Safari.words("The user approved Safari's connection, but Safari has not run the Vibe Work extension since. Ask the user to click {yourBrowser} in Safari's toolbar to finish.");
    }
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
                ConnectionError::Container => "Safari could not prepare its connection data. Ask the user to click Set up again. If it still fails, ask the user to reinstall Vibe Work.",
                ConnectionError::Relay => "Safari could not start its connection. Ask the user to click Set up again. If it still fails, ask the user to restart Vibe Work.",
            }.into();
        }
        finishing(shared, &mut addon, enabled);
        let fingerprint = matches!(addon.state, AddonState::NotPaired | AddonState::OtherOwner).then(|| pairing.1.clone()).flatten();
        let other_app_copies = shared.chrome_control.safari_copies.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        SafariView { enabled, package, addon, fingerprint, other_app_copies: (!other_app_copies.is_empty()).then_some(other_app_copies) }
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
    let previous = std::mem::replace(
        &mut *shared
            .chrome_control
            .owner
            .safari_pairing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        (state.into(), fingerprint),
    );
    // The user completed pairing in this run: the extension reported `waiting` (or another owner's `mismatch`)
    // and now `paired`. An already paired Safari reconnecting reports `paired` directly and never prompts, so the
    // system Accessibility prompt appears once per pairing, never at app start and never from an agent action.
    if state == "paired" && matches!(previous.0.as_str(), "waiting" | "mismatch") {
        accessibility_prompt(shared);
    }
    // A pairing state reaches the helper only from a connected extension (`safari_package::open` waits on this).
    shared
        .chrome_control
        .owner
        .safari_seen
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *shared
        .chrome_control
        .owner
        .safari_seen_at
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(std::time::Instant::now());
    super::owner::publish(shared);
}

/// Safari runs the extension's background page only briefly after its last native exchange, and Apple drops the
/// pairing notice while that page is not running (and still reports success). A waiting extension reports its
/// pairing state each time its page starts, so a report within this window means the notice can still arrive.
#[cfg(any(target_os = "macos", all(test, feature = "your-safari")))]
pub(super) const NOTICE_WINDOW: std::time::Duration = std::time::Duration::from_secs(20);

/// Whether the pairing notice can reach the extension: Safari is being launched for it (the notice then waits for the
/// extension's first report), or the extension reported within [`NOTICE_WINDOW`].
#[cfg(any(target_os = "macos", all(test, feature = "your-safari")))]
pub(super) fn notice_reachable(
    launching: bool,
    last_report: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    launching || last_report.is_some_and(|at| now.saturating_duration_since(at) < NOTICE_WINDOW)
}

/// Native Safari keys and clicks need Accessibility; ask macOS for its own prompt while the user is pairing.
/// The system call runs off the helper pipe loop. Tests count the request without calling the system.
fn accessibility_prompt(shared: &Shared) {
    shared
        .chrome_control
        .owner
        .safari_accessibility_prompts
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    #[cfg(all(not(test), target_os = "macos", feature = "your-safari"))]
    std::thread::spawn(|| {
        let _ = vw_platform::safari_input::request_accessibility();
    });
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
    chrome.owner.approvals.cancel_browser(Browser::Safari);
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
            hint: None,
        })
    }

    pub(super) async fn safari_remove(&self) -> Result<CommandResult, CoreError> {
        if !cfg!(all(feature = "your-safari", target_os = "macos")) {
            return Err(super::commands::invalid(&absent()));
        }
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        super::safari_pairing::revoke(&self.shared).await?;
        super::profiles::remove_browser(chrome, Browser::Safari);
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
            hint: None,
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
        let hint = super::safari_package::open(&self.shared)
            .map_err(|reason| super::commands::invalid(&reason))?;
        #[cfg(not(target_os = "macos"))]
        let hint = None;
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
            hint,
        })
    }
}

#[cfg(test)]
#[path = "safari_unit_tests.rs"]
mod tests;
