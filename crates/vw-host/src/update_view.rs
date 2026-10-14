//! What the update flow shows and what it needs from its surroundings: the serializable view the
//! window renders, the install mode, the requester of an action, the backend the Tauri crate implements with the
//! updater plugin, and the hooks for OS notifications and the restart.
//!
//! Wire form of the view (the UI mirrors it in `ui/src/protocol/client/types.ts`; a private test pins the JSON):
//! `{current, mode, autoDownload, releasePage, lastChecked, phase: {type: "idle"|"checking"|"available"|
//! "downloading"|"ready"|"installing"|"restarting"|"failed", ...}}`.
//!
//! Depends on: serde. Called by: [`crate::update_flow`], `app` (`updater.rs`), the private tests.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;

/// A boxed future, so a trait object can return async results.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Whether this installation can update itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallMode {
    /// `AppImage`, Windows installer or macOS app: download, ask, install in the app.
    InApp,
    /// A package-manager install (deb, rpm) or an unbundled run: tell about the new version and link to the
    /// release page; the app never downloads or installs (the package manager owns the files).
    LinkOnly,
}

impl InstallMode {
    /// The mode for the bundle this binary was built as (`tauri::utils::platform::bundle_type()` spelled as its
    /// `Display`: `appimage`, `msi`, `nsis`, `app`, `deb`, `rpm`, `dmg`), `None` for an unbundled binary. Only the
    /// formats the updater plugin can replace in place are [`InstallMode::InApp`]; a package-manager install and an
    /// unbundled run are [`InstallMode::LinkOnly`] (fail safe: unknown means link only).
    #[must_use]
    pub fn for_bundle(bundle: Option<&str>) -> Self {
        match bundle {
            Some("appimage" | "msi" | "nsis" | "app") => Self::InApp,
            _ => Self::LinkOnly,
        }
    }
}

/// Where every "Open release page" link goes: the official repository's release tags. The link is never taken from
/// the update manifest: `latest.json` is not signed as a whole (only the packages it lists
/// are), so a hostile channel could otherwise send a link-only user to a look-alike download page. The host shown
/// beside the button is this URL's host.
pub const OFFICIAL_RELEASE_TAGS: &str =
    "https://github.com/martinsbrezauckis/vibe-work/releases/tag/";

/// The official release page of `version` (`<OFFICIAL_RELEASE_TAGS>v<version>`, the tag the release step makes), or
/// `None` when `version` is not a canonical semver version (`1.2.3`, `1.2.3-rc.1`; no build metadata, no leading
/// `v`, nothing that would change the URL's path). The version comes from the unsigned manifest, so it is checked
/// before it becomes part of a link.
#[must_use]
pub fn release_page_for(version: &str) -> Option<String> {
    let parsed = semver::Version::parse(version).ok()?;
    // Canonical form only (rejects e.g. "01.2.3" spellings that parse differently elsewhere), and no build metadata:
    // tags never carry it and `+` is not part of the tag name.
    (parsed.build.is_empty() && parsed.to_string() == version)
        .then(|| format!("{OFFICIAL_RELEASE_TAGS}v{version}"))
}

impl UpdateNotice {
    /// Title and body of the OS notification, in plain words.
    #[must_use]
    pub fn text(&self) -> (String, String) {
        match self {
            Self::Available {
                version,
                downloading: true,
                ..
            } => (
                "Vibe Work update available".to_owned(),
                format!(
                    "Version {version} is downloading. Vibe Work will ask before it installs anything."
                ),
            ),
            Self::Available {
                version,
                link_only: true,
                ..
            } => (
                "Vibe Work update available".to_owned(),
                format!("Version {version} is available. Open Vibe Work to see where to get it."),
            ),
            Self::Available { version, .. } => (
                "Vibe Work update available".to_owned(),
                format!("Version {version} is available. Open Vibe Work to download it."),
            ),
            Self::Ready { version } => (
                "Vibe Work update ready".to_owned(),
                format!("Version {version} is downloaded. Open Vibe Work to install and restart."),
            ),
        }
    }
}

impl From<crate::update_flow::UpdateError> for vw_core_api::CoreError {
    fn from(error: crate::update_flow::UpdateError) -> Self {
        use crate::update_flow::UpdateError as E;
        let code = match &error {
            E::Failed(_) => vw_core_api::ErrorCode::Internal,
            E::Busy | E::WrongState | E::ConsentRequired | E::LinkOnly => {
                vw_core_api::ErrorCode::Denied
            }
        };
        Self::new(code, error.to_string())
    }
}

/// Which step failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    /// Asking the update server.
    Check,
    /// Downloading or verifying the package.
    Download,
    /// Installing it.
    Install,
}

/// Where the flow is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Phase {
    /// Nothing to offer (up to date, or not checked yet).
    Idle,
    /// Asking the update server.
    Checking,
    /// A newer version exists; not downloaded (link-only mode, or the user chose to be asked first).
    Available {
        /// The announced version.
        version: String,
        /// Release notes, when the manifest has them.
        notes: Option<String>,
    },
    /// Downloading in the background.
    Downloading {
        /// The version being downloaded.
        version: String,
        /// Release notes.
        notes: Option<String>,
        /// Bytes received.
        received: u64,
        /// Total bytes when the server said.
        total: Option<u64>,
    },
    /// Downloaded and verified; waiting for the user's "install and restart".
    Ready {
        /// The version that will be installed.
        version: String,
        /// Release notes.
        notes: Option<String>,
    },
    /// Installing (the user agreed).
    Installing {
        /// The version being installed.
        version: String,
    },
    /// Installed; the app is restarting.
    Restarting {
        /// The installed version.
        version: String,
    },
    /// A step failed; the offer, if any, is kept and "Try again" runs the check again.
    Failed {
        /// Which step.
        stage: Stage,
        /// Plain-words reason.
        error: String,
        /// The version concerned, when known.
        version: Option<String>,
    },
}

/// Everything the window needs to render the update notice and the Settings section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    /// The running version.
    pub current: String,
    /// In-app or link-only.
    pub mode: InstallMode,
    /// The "Download updates automatically" setting.
    pub auto_download: bool,
    /// The official release page of the offered version ([`release_page_for`]); never a link from the manifest.
    pub release_page: Option<String>,
    /// Unix seconds of the last finished check.
    pub last_checked: Option<u64>,
    /// Where the flow is.
    pub phase: Phase,
}

/// What an action's caller is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateRequester {
    /// The main window (a click on a trusted surface). Only the app crate's window command builds this.
    Window,
    /// An external client through the Debug API (acts as the user; logged).
    DebugApi,
}

/// A newer version the server announced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// The announced version (semver).
    pub version: String,
    /// Release notes.
    pub notes: Option<String>,
    /// The announced signature (base64 minisign text).
    pub signature: String,
}

/// Download progress callback: `(bytes received so far, total when known)`.
pub type Progress = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;

/// The network side, implemented in the Tauri crate with the updater plugin and by scripted fakes in tests.
pub trait UpdateBackend: Send + Sync {
    /// Asks the server for a version newer than the running one; `Ok(None)` when up to date.
    fn check(&self) -> BoxFuture<Result<Option<Offer>, String>>;
    /// Downloads the package (the plugin verifies its signature as it finishes; the flow verifies again).
    fn download(&self, offer: Offer, progress: Progress) -> BoxFuture<Result<Vec<u8>, String>>;
    /// Installs a verified package. Linux/macOS return afterwards; Windows may exit the process.
    fn install(&self, offer: Offer, package: Vec<u8>) -> BoxFuture<Result<(), String>>;
}

/// An OS notification the flow wants shown (the app crate maps it to text and the desktop notifier).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateNotice {
    /// A newer version exists. `downloading`: the app will fetch it now.
    Available {
        /// The version.
        version: String,
        /// Whether the download starts by itself.
        downloading: bool,
        /// Link-only install: the notice is the only offer.
        link_only: bool,
    },
    /// The update is downloaded and verified; it asks to install.
    Ready {
        /// The version.
        version: String,
    },
}

/// Side effects the flow asks of the app.
pub trait UpdateHooks: Send + Sync {
    /// Shows an OS notification.
    fn notify(&self, notice: UpdateNotice);
    /// Relaunches the app after an install (Linux, macOS): shuts the core down, then restarts the process.
    fn restart(&self) -> BoxFuture<()>;
}
