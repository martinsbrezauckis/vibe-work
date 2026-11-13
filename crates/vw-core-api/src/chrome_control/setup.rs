//! Installs the owned helper and Chrome's per-user native registration. Depends on std and serde;
//! called by Chrome Settings. No agent plugin or launcher is installed.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Chrome Control's native messaging host name (the helper registers under it; the extension connects to it).
pub const HOST_NAME: &str = "com.vibework.chrome_control";
/// The extension's id in the Chrome Web Store: the store item's id, derived from its public key.
pub const FIREFOX_EXTENSION_ID: &str = "browser-control@vibework.app";
/// The Chrome Web Store extension identity.
pub const EXTENSION_ID: &str = "gnlamocfdabihichplpepjpehlbafhng";
/// The extension's name in the Chrome Web Store and in Chrome (the app's own name: say "the Vibe Work extension" where
/// text could mean the app). Google's trademark rule
/// keeps "Chrome" out of the name (the listing says "for Google Chrome"); the same family name will serve other
/// browsers later. The one place to change it on Vibe Work's side; Vibe Work's Settings section stays "Your Chrome".
pub const STORE_LISTING_NAME: &str = "Vibe Work";
/// The Mozilla-signed Firefox sharing add-on inside the app's resources (release bundles only).
pub const FIREFOX_ADDON_RESOURCE: &str = "firefox-addon/vibe-work.xpi";
/// The Chrome Control installation offer key.
pub const PLUGIN: &str = "chrome-control";

/// Opens a web address in the user's Google Chrome (tests record it instead).
pub type Launcher = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

#[cfg(test)]
type AfterHelperCheck = Arc<dyn Fn(&Path) + Send + Sync>;

/// Where Chrome Control comes from and where Chrome looks for it.
#[derive(Clone)]
pub struct ChromeControlConfig {
    /// Native Chrome/Edge facts cached independently of pairing; probes run away from async workers.
    pub browser_detection: vw_platform::browser::Detection,
    /// Browser-local permission failures; reset only after successful setup or removal.
    pub setup_problems: SetupProblems,
    #[cfg(test)]
    pub(crate) macos: Option<bool>,
    #[cfg(test)]
    pub(super) file_access: Option<files::AccessCheck>,
    /// The trusted native package shipped with Vibe Work (`bin/chrome-control`, `.exe` on Windows).
    /// `None`: this build has none, and Set up says so.
    pub package: Option<PathBuf>,
    #[cfg(test)]
    pub(super) after_helper_check: Option<AfterHelperCheck>,
    #[cfg(test)]
    pub(super) edge_policy: Option<serde_json::Value>,
    /// Records the System Settings address the profile-name button opens, in place of opening it.
    #[cfg(test)]
    pub(super) settings_opener: Option<Launcher>,
    /// SHA-256 of the final shipped helper, embedded by the release build.
    pub helper_sha256: Option<String>,
    /// Kernel code-directory hash of the signed macOS helper, embedded by the release build.
    #[cfg(target_os = "macos")]
    pub helper_cdhash: Option<String>,
    /// The store extension's id.
    pub extension_id: String,
    /// The user's home folder for Chrome's per-user places; `None` = this user's real home. Tests give a scratch one.
    pub user_home: Option<PathBuf>,
    /// Windows native-host registry root. Defaults to Chrome's per-user key; tests use a disposable root.
    pub windows_registry_root: String,
    /// Edge per-user registration root; used only with the optional browser module.
    pub edge_registry_root: String,
    /// Mozilla per-user native host key.
    pub firefox_registry_root: String,
    /// Opens the store page in Chrome; `None` = start Google Chrome with the address.
    pub launcher: Option<Launcher>,
    /// Opens the same store listing in Edge; used by the optional browser module.
    pub edge_launcher: Option<Launcher>,
    /// Opens the Firefox add-on page.
    pub firefox_launcher: Option<Launcher>,
    /// Approved Mozilla listing supplied by the later-release package; none before publication.
    pub firefox_store_url: Option<String>,
    /// SHA-256 of the Mozilla-signed Firefox sharing add-on shipped in the bundle, embedded by the release build.
    pub firefox_addon_sha256: Option<String>,
    /// The bundled signed add-on file (`firefox-addon/vibe-work.xpi` in the resources); `None` without a pin.
    pub firefox_addon: Option<PathBuf>,
}

impl Default for ChromeControlConfig {
    fn default() -> Self {
        Self {
            browser_detection: vw_platform::browser::Detection::default(),
            setup_problems: SetupProblems::default(),
            #[cfg(test)]
            macos: None,
            #[cfg(test)]
            file_access: None,
            package: None,
            #[cfg(test)]
            after_helper_check: None,
            #[cfg(test)]
            edge_policy: None,
            #[cfg(test)]
            settings_opener: None,
            helper_sha256: option_env!("VW_CHROME_HELPER_SHA256").map(str::to_owned),
            #[cfg(target_os = "macos")]
            helper_cdhash: option_env!("VW_CHROME_HELPER_CDHASH").map(str::to_owned),
            extension_id: EXTENSION_ID.to_owned(),
            user_home: None,
            windows_registry_root: r"HKCU\Software\Google\Chrome\NativeMessagingHosts".to_owned(),
            edge_registry_root: r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts".to_owned(),
            launcher: None,
            edge_launcher: None,
            firefox_launcher: None,
            firefox_store_url: None,
            firefox_addon_sha256: option_env!("VW_FIREFOX_ADDON_SHA256").map(str::to_owned),
            firefox_addon: None,
            firefox_registry_root: r"HKCU\Software\Mozilla\NativeMessagingHosts".to_owned(),
        }
    }
}

impl ChromeControlConfig {
    // The instance supplies the permission-test platform; production uses the build target.
    #[allow(clippy::unused_self)]
    pub(super) fn is_macos(&self) -> bool {
        #[cfg(test)]
        if let Some(macos) = self.macos {
            return macos;
        }
        cfg!(target_os = "macos")
    }

    /// Package presence is the feature gate; a damaged shipped package remains present and reports its error.
    #[must_use]
    pub fn available(&self) -> bool {
        self.package.is_some()
    }

    /// A release without a helper pin has no package; a pinned but missing helper needs repair.
    /// The signed Firefox add-on follows its own pin the same way.
    pub fn bundled(&mut self, resources: &Path) {
        self.package = self
            .helper_sha256
            .as_ref()
            .map(|_| resources.join("chrome-control"));
        self.firefox_addon = self
            .firefox_addon_sha256
            .as_ref()
            .map(|_| resources.join(FIREFOX_ADDON_RESOURCE));
    }
}

/// Plain refusal shared by unavailable Settings and tool paths.
pub(crate) const ABSENT: &str = vw_agent_tools::wire::CHROME_UNAVAILABLE;

pub(super) fn absent_for(edge: bool) -> &'static str {
    if edge {
        vw_vocabulary::texts::EDGE_UNAVAILABLE
    } else {
        ABSENT
    }
}

impl std::fmt::Debug for ChromeControlConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChromeControlConfig")
            .field("package", &self.package)
            .field("extension_id", &self.extension_id)
            .field("user_home", &self.user_home)
            .field("windows_registry_root", &self.windows_registry_root)
            .finish_non_exhaustive()
    }
}

/// The store page of the extension.
#[must_use]
pub fn store_url(extension_id: &str) -> String {
    format!("https://chromewebstore.google.com/detail/{extension_id}")
}

/// Where Chrome Control stands, from files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", ts(rename = "ChromeSetupState"))]
pub enum SetupState {
    /// This build does not carry Chrome Control.
    Absent,
    /// Nothing is installed.
    NotSetUp,
    /// A recorded native host could not be restored; retry setup.
    RepairRequired,
    /// The helper is installed; no Chrome profile has the extension yet.
    WaitingForExtension,
    /// The helper is installed and at least one Chrome profile has the extension.
    ExtensionAdded,
}

/// What Settings shows about the setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChromeSetupView {
    /// Where it stands.
    pub state: SetupState,
    /// This build ships the package, so Set up can run.
    pub package: bool,
    /// The browsers the helper is registered with ("Google Chrome").
    pub browsers: Vec<String>,
    /// The Chrome profiles whose folder has the extension ("Default", "Profile 1").
    pub profiles: Vec<String>,
    /// The store page.
    pub store_url: String,
    /// The store listing is in Google's review (the last accepted signed release manifest says so): the setup shows
    /// that in place of the store button. False when no manifest stated it (the store link).
    pub store_review: bool,
    /// Typed helper observation; unknown in views saved before this field existed.
    #[serde(default)]
    pub pairing_state: super::PairingState,
    /// Display-only pairing label. Connection decisions use `pairing_state`.
    pub pairing: String,
    /// Short public identity to compare with Chrome (never the owner secret).
    pub fingerprint: Option<String>,
    /// A detected unsupported or missing browser; absent in older saved views.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub browser_issue: Option<super::browser::BrowserIssue>,
    /// Installation route: external consent/restart, or Store with extension facts reported after pairing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub install_state: Option<String>,
    /// Browser-reported permission states, absent for existing browser views.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub states: Option<Vec<String>>,
    /// The one Your Chrome state the card and the agent share (label, action, instruction).
    /// Always set by the core; absent only in older saved views.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub addon: Option<super::addon::AddonView>,
    /// After Remove: the extension is still in a Chrome profile (Chrome did not remove it by itself), so the person
    /// removes it in Chrome too. Absent: nothing is known to be left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub extension_left: Option<bool>,
    /// After Remove, Chrome must restart to finish removing its externally installed extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub extension_restart: Option<bool>,
    /// macOS refused reading this browser's profile folder for a paired profile, so its profiles keep numbered
    /// names: the card says how to allow it in System Settings. Absent: nothing refused (always on Windows and Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub profile_names_refused: Option<bool>,
}

/// What a set up made, so Remove deletes exactly that.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Receipt {
    files: Vec<PathBuf>,
    browsers: Vec<String>,
    /// Chrome user data folders the helper is registered for.
    user_data: Vec<PathBuf>,
    #[serde(default)]
    registry_keys: Vec<String>,
    /// Explicit Connect choices; absent in receipts that could automatically include both browsers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected_browsers: Option<Vec<String>>,
}

impl Receipt {
    fn data_for(&self, browser: &str) -> Vec<PathBuf> {
        self.user_data
            .iter()
            .enumerate()
            .filter(|(index, dir)| {
                self.browsers
                    .get(*index)
                    .map(String::as_str)
                    .or_else(|| places::browser_of(dir))
                    == Some(browser)
            })
            .map(|(_, dir)| dir.clone())
            .collect()
    }
}

const RECEIPT: &str = "setup.json";

#[path = "setup_problem.rs"]
mod problem;
pub use problem::SetupProblems;

#[path = "setup_files.rs"]
pub(super) mod files;
#[path = "setup_registration.rs"]
mod registration;
#[path = "setup_selection.rs"]
mod selection;

pub(super) fn migrate_selections(config: &ChromeControlConfig, state: &Path) -> Result<(), String> {
    let _held = registration_lock();
    selection::migrate(config, state)
}

static REGISTRATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn registration_lock() -> std::sync::MutexGuard<'static, ()> {
    REGISTRATION_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn repair_registrations(
    config: &ChromeControlConfig,
    state: &Path,
    browser: Option<&str>,
) -> Result<(), String> {
    let _held = registration_lock();
    let result = registration::repair(config, state, browser);
    if let Some(browser) = browser {
        problem::finish(config, state, browser, result)
    } else {
        result
    }
}

pub(super) fn helper_name() -> &'static str {
    if cfg!(windows) {
        "chrome-control.exe"
    } else {
        "chrome-control"
    }
}

#[path = "setup_targets.rs"]
mod places;
use places::targets;

pub(super) fn home_of(config: &ChromeControlConfig) -> Result<PathBuf, String> {
    config
        .user_home
        .clone()
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
        })
        .filter(|h| h.is_absolute())
        .ok_or_else(|| "your home folder could not be found".to_owned())
}

fn read_receipt(config: &ChromeControlConfig, state: &Path) -> Option<Receipt> {
    let bytes = files::read(config, &state.join(RECEIPT)).ok()??;
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn registered(
    config: &ChromeControlConfig,
    state: Option<&Path>,
    browser: &str,
) -> bool {
    state.is_some_and(|state| {
        read_receipt(config, state).is_some_and(|receipt| {
            receipt.browsers.iter().any(|name| name == browser)
                && selection::selected(config, state, &receipt, browser)
        })
    })
}

/// The files Set up wrote (helper copy and manifests); `None` before Set up.
pub(super) fn receipt_files(config: &ChromeControlConfig, state: &Path) -> Option<Vec<PathBuf>> {
    read_receipt(config, state).map(|receipt| receipt.files)
}

/// The Chrome profiles under `user_data` that have the extension's folder.
fn profiles_with(
    config: &ChromeControlConfig,
    user_data: &Path,
    extension_id: &str,
) -> Vec<String> {
    let mut found: Vec<String> = super::probe::profiles_for(config, user_data, extension_id)
        .iter()
        .filter_map(|p| p.file_name()?.to_str().map(str::to_owned))
        .collect();
    found.sort();
    found
}

/// The status Settings shows.
pub(crate) fn view(
    config: &ChromeControlConfig,
    state: Option<&Path>,
    vibe_home: &Path,
) -> ChromeSetupView {
    view_for(config, state, vibe_home, "Google Chrome")
}

pub(super) fn view_for(
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
            pairing_state: super::PairingState::Unknown,
            pairing: ABSENT.to_owned(),
            fingerprint: None,
            browser_issue: None,
            install_state: None,
            states: None,
            addon: Some(super::addon::view(
                super::addon::AddonState::Unavailable,
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
        pairing_state: super::PairingState::Unknown,
        pairing: if repair_failed {
            "Helper setup required".into()
        } else {
            package
                .err()
                .unwrap_or_else(|| vw_vocabulary::texts::CHROME_NOT_CONNECTED.to_owned())
        },
        fingerprint: None,
        browser_issue: match browser {
            "Google Chrome" => super::browser::issue(config),
            "Microsoft Edge" => {
                super::browser::issue_for(config, vw_platform::browser::Browser::Edge)
            }
            _ => None,
        },
        install_state: if config.is_macos() {
            Some("store".into())
        } else {
            (browser == "Google Chrome")
                .then(|| state.and_then(|state| super::extension_install::status(config, state)))
                .flatten()
        },
        states: None,
        // A refused attempt stays visible without retrying it on every status poll.
        addon: (blocked || problem::blocked(config, state, browser)).then(|| {
            super::addon::view_for(
                super::addon::AddonState::AccessBlocked,
                "",
                if browser == "Microsoft Edge" {
                    super::agent_browser::Browser::Edge
                } else {
                    super::agent_browser::Browser::Chrome
                },
            )
        }),
        extension_left: None,
        extension_restart: None,
        profile_names_refused: None,
    }
}

fn needs_repair(
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

/// Only this browser's registered folders. Receipt identities take precedence over known vendor paths.
pub(super) fn user_data(
    config: &ChromeControlConfig,
    state: Option<&Path>,
    browser: &str,
) -> Vec<PathBuf> {
    state
        .and_then(|state| read_receipt(config, state))
        .map(|receipt| receipt.data_for(browser))
        .unwrap_or_default()
}

/// Reads the helper once and checks the release pin before copying or starting it.
pub(super) fn packaged_helper(config: &ChromeControlConfig) -> Result<(PathBuf, Vec<u8>), String> {
    use std::io::Read;
    let (mut opened, path) = open_helper(config)?;
    let mut bytes = Vec::new();
    opened
        .file()
        .read_to_end(&mut bytes)
        .map_err(|_| missing_helper())?;
    check_helper_pin(config, &bytes)?;
    Ok((path, bytes))
}

fn missing_helper() -> String {
    vw_vocabulary::texts::CHROME_PACKAGE_MISSING.to_owned()
}

pub(super) fn open_helper(
    config: &ChromeControlConfig,
) -> Result<(vw_platform::checked_executable::CheckedExecutable, PathBuf), String> {
    let package = config.package.as_ref().ok_or_else(|| ABSENT.to_owned())?;
    let path = package.join("bin").join(helper_name());
    let opened = vw_platform::checked_executable::CheckedExecutable::open(&path)
        .map_err(|_| missing_helper())?;
    Ok((opened, path))
}

pub(super) fn check_helper_pin(config: &ChromeControlConfig, bytes: &[u8]) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    match config.helper_sha256.as_deref() {
        Some(pin) if format!("{:x}", Sha256::digest(bytes)) == pin => {}
        None if cfg!(debug_assertions) => {}
        _ => {
            return Err(vw_vocabulary::texts::CHROME_PACKAGE_CHANGED.to_owned());
        }
    }
    Ok(())
}

/// What was made so far, undone in reverse when a later step fails.
#[derive(Default)]
struct Made {
    files: Vec<PathBuf>,
    keys: Vec<(String, PathBuf)>,
    written: Vec<(PathBuf, Vec<u8>)>,
}

impl Made {
    fn undo(&self, config: &ChromeControlConfig) {
        for (key, file) in self.keys.iter().rev() {
            let _ = registry_delete(key, file);
        }
        for (file, bytes) in self.written.iter().rev() {
            let _ = files::remove(config, file, bytes);
        }
    }
}

fn write_file(
    config: &ChromeControlConfig,
    made: &mut Made,
    path: &Path,
    bytes: &[u8],
) -> Result<(), String> {
    let before = files::read(config, path).map_err(|e| problem::io_message(config, &e))?;
    files::write(config, path, before.as_deref(), bytes)
        .map_err(|e| problem::io_message(config, &e))?;
    made.files.push(path.to_path_buf());
    made.written.push((path.to_path_buf(), bytes.to_vec()));
    Ok(())
}

/// The native messaging manifest for the helper at `helper`.
pub(super) fn host_manifest(helper: &Path, extension_id: &str) -> Vec<u8> {
    let helper = if cfg!(windows) {
        PathBuf::from(windows_native_path(helper))
    } else {
        helper.to_path_buf()
    };
    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": format!("{STORE_LISTING_NAME} extension helper"),
        "path": helper,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{extension_id}/")],
    });
    serde_json::to_vec_pretty(&manifest).unwrap_or_default()
}

#[path = "setup_install.rs"]
mod installing;
#[cfg(test)]
pub(crate) use installing::install;
pub(super) use installing::install_for;

#[path = "setup_remove.rs"]
mod removal;
pub(crate) use removal::remove;
#[path = "setup_scoped_remove.rs"]
mod scoped_removal;
pub(super) use scoped_removal::{has_other, remove_browser};

/// Opens the store page in Google Chrome.
pub(crate) fn open_store(config: &ChromeControlConfig) -> Result<(), String> {
    open_store_for(config, false)
}

pub(super) fn open_store_for(config: &ChromeControlConfig, edge: bool) -> Result<(), String> {
    if !edge {
        return super::browser::open(config, &store_url(&config.extension_id));
    }
    super::browser::open_edge(config, &store_url(&config.extension_id))
}

fn registry_set(key: &str, file: &Path) -> Result<(), String> {
    super::extension_registry::host(
        key,
        &windows_native_path(file),
        false,
        &super::extension_registry::Native,
    )
}

// Native messaging expects backslashes even when a configured root uses forward slashes.
fn windows_native_path(file: &Path) -> String {
    file.to_string_lossy().replace('/', "\\")
}

fn registry_delete(key: &str, file: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Ok(());
    }
    super::extension_registry::remove_host(
        key,
        &windows_native_path(file),
        &super::extension_registry::Native,
    )
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;

#[path = "setup_edge.rs"]
mod edge;
pub(super) use edge::register_edge;

#[path = "setup_firefox.rs"]
pub(super) mod firefox;

fn manifest_for(helper: &Path, config: &ChromeControlConfig, browser: &str) -> Vec<u8> {
    if browser == "Mozilla Firefox" {
        firefox::manifest(helper)
    } else {
        host_manifest(helper, &config.extension_id)
    }
}
fn manifest_path(state: &Path, browser: &str) -> PathBuf {
    state
        .join(if browser == "Mozilla Firefox" {
            "firefox-host"
        } else {
            "host"
        })
        .join(format!("{HOST_NAME}.json"))
}
