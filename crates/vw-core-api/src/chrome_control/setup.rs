//! Installs the owned helper and Chrome's per-user native registration. Depends on std and serde;
//! called by Chrome Settings. No agent plugin or launcher is installed.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Chrome Control's native messaging host name (the helper registers under it; the extension connects to it).
pub const HOST_NAME: &str = "com.vibework.chrome_control";
/// The extension's id in the Chrome Web Store: the store item's id, derived from its public key.
pub const EXTENSION_ID: &str = "gdkmdhknnlblijooalnfaenjabeifadd";
/// The extension's name in the Chrome Web Store and in Chrome. Google's trademark rule
/// keeps "Chrome" out of the name (the listing says "for Google Chrome"); the same family name will serve other
/// browsers later. The one place to change it on Vibe Work's side; Vibe Work's Settings section stays "Your Chrome".
pub const STORE_LISTING_NAME: &str = "Vibe Work Browser Control";
/// The Chrome Control installation offer key.
pub const PLUGIN: &str = "chrome-control";

/// Opens a web address in the user's Google Chrome (tests record it instead).
pub type Launcher = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

#[cfg(test)]
type AfterHelperCheck = Arc<dyn Fn(&Path) + Send + Sync>;

/// Where Chrome Control comes from and where Chrome looks for it.
#[derive(Clone)]
pub struct ChromeControlConfig {
    /// The trusted native package shipped with Vibe Work (`bin/chrome-control`, `.exe` on Windows).
    /// `None`: this build has none, and Set up says so.
    pub package: Option<PathBuf>,
    #[cfg(test)]
    pub(super) after_helper_check: Option<AfterHelperCheck>,
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
    /// Opens the store page in Chrome; `None` = start Google Chrome with the address.
    pub launcher: Option<Launcher>,
}

impl Default for ChromeControlConfig {
    fn default() -> Self {
        Self {
            package: None,
            #[cfg(test)]
            after_helper_check: None,
            helper_sha256: option_env!("VW_CHROME_HELPER_SHA256").map(str::to_owned),
            #[cfg(target_os = "macos")]
            helper_cdhash: option_env!("VW_CHROME_HELPER_CDHASH").map(str::to_owned),
            extension_id: EXTENSION_ID.to_owned(),
            user_home: None,
            windows_registry_root: r"HKCU\Software\Google\Chrome\NativeMessagingHosts".to_owned(),
            launcher: None,
        }
    }
}

impl ChromeControlConfig {
    /// Package presence is the feature gate; a damaged shipped package remains present and reports its error.
    #[must_use]
    pub fn available(&self) -> bool {
        self.package.is_some()
    }

    /// A release without a helper pin has no package; a pinned but missing helper needs repair.
    pub fn bundled(&mut self, resources: &Path) {
        self.package = self
            .helper_sha256
            .as_ref()
            .map(|_| resources.join("chrome-control"));
    }
}

/// Plain refusal shared by unavailable Settings and tool paths.
pub(crate) const ABSENT: &str = vw_agent_tools::wire::CHROME_UNAVAILABLE;

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
    /// Pairing state reported by the owned helper, in plain words.
    pub pairing: String,
    /// Short public identity to compare with Chrome (never the owner secret).
    pub fingerprint: Option<String>,
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
}

const RECEIPT: &str = "setup.json";

pub(super) fn helper_name() -> &'static str {
    if cfg!(windows) {
        "chrome-control.exe"
    } else {
        "chrome-control"
    }
}

/// One Chrome this user may run: its name, its user data folder, its per-user host folder (`None` on Windows,
/// where the registry names the manifest).
struct Target<'a> {
    name: &'static str,
    user_data: PathBuf,
    hosts: Option<PathBuf>,
    registry: Option<&'a str>,
}

fn targets<'a>(home: &Path, config: &'a ChromeControlConfig) -> Vec<Target<'a>> {
    if cfg!(target_os = "macos") {
        let base = home.join("Library/Application Support/Google/Chrome");
        vec![Target {
            name: "Google Chrome",
            hosts: Some(base.join("NativeMessagingHosts")),
            user_data: base,
            registry: None,
        }]
    } else if cfg!(windows) {
        vec![Target {
            name: "Google Chrome",
            user_data: home
                .join("AppData")
                .join("Local")
                .join("Google")
                .join("Chrome")
                .join("User Data"),
            hosts: None,
            registry: Some(&config.windows_registry_root),
        }]
    } else {
        let base = home.join(".config/google-chrome");
        vec![Target {
            name: "Google Chrome",
            hosts: Some(base.join("NativeMessagingHosts")),
            user_data: base,
            registry: None,
        }]
    }
}

fn home_of(config: &ChromeControlConfig) -> Result<PathBuf, String> {
    config
        .user_home
        .clone()
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
        })
        .filter(|h| h.is_absolute())
        .ok_or_else(|| "your home folder could not be found".to_owned())
}

fn read_receipt(state: &Path) -> Option<Receipt> {
    let bytes = std::fs::read(state.join(RECEIPT)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The Chrome profiles under `user_data` that have the extension's folder.
fn profiles_with(user_data: &Path, extension_id: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(user_data) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().join("Extensions").join(extension_id).is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect();
    found.sort();
    found
}

/// The status Settings shows.
pub(crate) fn view(
    config: &ChromeControlConfig,
    state: Option<&Path>,
    _vibe_home: &Path,
) -> ChromeSetupView {
    if !config.available() {
        return ChromeSetupView {
            state: SetupState::Absent,
            package: false,
            browsers: Vec::new(),
            profiles: Vec::new(),
            store_url: String::new(),
            pairing: ABSENT.to_owned(),
            fingerprint: None,
        };
    }
    let receipt = state.and_then(read_receipt);
    let profiles: Vec<String> = receipt
        .iter()
        .flat_map(|r| r.user_data.iter())
        .flat_map(|dir| profiles_with(dir, &config.extension_id))
        .collect();
    let package = packaged_helper(config);
    ChromeSetupView {
        state: match (&receipt, profiles.is_empty()) {
            (None, _) => SetupState::NotSetUp,
            (Some(_), true) => SetupState::WaitingForExtension,
            (Some(_), false) => SetupState::ExtensionAdded,
        },
        package: package.is_ok(),
        browsers: receipt.map(|r| r.browsers).unwrap_or_default(),
        profiles,
        store_url: store_url(&config.extension_id),
        pairing: package
            .err()
            .unwrap_or_else(|| "Chrome Control is not connected".to_owned()),
        fingerprint: None,
    }
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
    "This Vibe Work installation has no Chrome Control helper. Reinstall Vibe Work.".to_owned()
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
        _ => return Err("The Chrome helper in this installation is not the one that shipped with Vibe Work. Reinstall Vibe Work.".to_owned()),
    }
    Ok(())
}

/// What was made so far, undone in reverse when a later step fails.
#[derive(Default)]
struct Made {
    files: Vec<PathBuf>,
    keys: Vec<String>,
}

impl Made {
    fn undo(&self) {
        for key in self.keys.iter().rev() {
            let _ = registry_delete(key);
        }
        for file in self.files.iter().rev() {
            let _ = std::fs::remove_file(file);
        }
    }
}

fn write_file(made: &mut Made, path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let temp = path.with_extension("new");
    std::fs::write(&temp, bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("{}: {e}", path.display())
    })?;
    made.files.push(path.to_path_buf());
    Ok(())
}

/// The native messaging manifest for the helper at `helper`.
fn host_manifest(helper: &Path, extension_id: &str) -> Vec<u8> {
    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": format!("{STORE_LISTING_NAME} helper"),
        "path": helper,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{extension_id}/")],
    });
    serde_json::to_vec_pretty(&manifest).unwrap_or_default()
}

/// Sets up Chrome Control (see the module docs). On error nothing stays changed.
pub(crate) fn install(
    config: &ChromeControlConfig,
    state: &Path,
    vibe_home: &Path,
) -> Result<ChromeSetupView, String> {
    let (_, bytes) = packaged_helper(config)?;
    let home = home_of(config)?;
    let mut made = Made::default();
    let result = (|| -> Result<Receipt, String> {
        let helper = state.join("bin").join(helper_name());
        write_file(&mut made, &helper, &bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let manifest = host_manifest(&helper, &config.extension_id);
        let mut receipt = Receipt::default();
        for target in targets(&home, config) {
            match (&target.hosts, target.registry) {
                (Some(hosts), _) => {
                    write_file(
                        &mut made,
                        &hosts.join(format!("{HOST_NAME}.json")),
                        &manifest,
                    )?;
                }
                (None, Some(root)) => {
                    let file = state.join("host").join(format!("{HOST_NAME}.json"));
                    write_file(&mut made, &file, &manifest)?;
                    let key = format!(r"{root}\{HOST_NAME}");
                    registry_set(&key, &file)?;
                    made.keys.push(key.clone());
                    receipt.registry_keys.push(key);
                }
                (None, None) => {}
            }
            receipt.browsers.push(target.name.to_owned());
            receipt.user_data.push(target.user_data);
        }
        receipt.files.clone_from(&made.files);
        let bytes = serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?;
        write_file(&mut made, &state.join(RECEIPT), &bytes)?;
        Ok(receipt)
    })();
    match result {
        Ok(_) => Ok(view(config, Some(state), vibe_home)),
        Err(problem) => {
            made.undo();
            Err(problem)
        }
    }
}

/// Removes what the receipt lists and the whole state folder.
pub(crate) fn remove(state: &Path) -> Result<(), String> {
    let receipt = read_receipt(state).unwrap_or_default();
    let mut problems = Vec::new();
    for key in &receipt.registry_keys {
        if let Err(e) = registry_delete(key) {
            problems.push(e);
        }
    }
    for file in &receipt.files {
        match std::fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                problems.push(format!("{}: {e}", file.display()));
            }
            _ => {}
        }
    }
    match std::fs::remove_dir_all(state) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            problems.push(format!("{}: {e}", state.display()));
        }
        _ => {}
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Opens the store page in Google Chrome.
pub(crate) fn open_store(config: &ChromeControlConfig) -> Result<(), String> {
    let url = store_url(&config.extension_id);
    if let Some(launcher) = &config.launcher {
        return launcher(&url);
    }
    let mut command = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.args(["-a", "Google Chrome", &url]);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", "chrome", &url]);
        c
    } else {
        let mut c = std::process::Command::new("google-chrome");
        c.arg(&url);
        c
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("Google Chrome could not be started: {e}"))
}

fn registry_set(key: &str, file: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("the registry exists on Windows only".to_owned());
    }
    let file = file.to_string_lossy();
    run_reg(&["add", key, "/ve", "/t", "REG_SZ", "/d", &file, "/f"])
}

fn registry_delete(key: &str) -> Result<(), String> {
    if !cfg!(windows) {
        return Ok(());
    }
    run_reg(&["delete", key, "/f"])
}

fn run_reg(args: &[&str]) -> Result<(), String> {
    let status = std::process::Command::new("reg")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("reg could not run: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "reg {} failed",
            args.first().copied().unwrap_or("")
        ))
    }
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
