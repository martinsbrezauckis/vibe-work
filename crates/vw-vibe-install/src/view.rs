//! What the setup screen, Settings and the Debug API see of the Vibe install: the offer the user
//! confirms, the phase, existing installs found on this machine. Plain data, no secret (there is none in an
//! install). The UI mirrors these types through `ui/src/protocol/core/vibe.ts` (generated, `ts` feature).
//!
//! Wire form: `{pinnedVersion, phase: {state: "notInstalled"|"offered"|"installing"|"failed"|"installed",...},
//! existing: [...], checkedExisting, update: {available, upcoming, mismatch, checkedAt, checkError, phase}}`.
//!
//! Depends on: serde. Called by: [`crate::manager`], vw-core-api (`vibe` commands and events).

use serde::{Deserialize, Serialize};

/// One official source the install downloads from, as the confirmation lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    /// What it is ("uv 0.12.24 (Astral)", "mistral-vibe 2.26.0 (`PyPI`)", "Python 3.14.8 (through uv)").
    pub name: String,
    /// Where it comes from (download or page URL).
    pub url: String,
    /// The SHA-256 it is checked against, when one file is downloaded by us.
    pub sha256: Option<String>,
    /// Size in bytes, when known.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub bytes: Option<u64>,
}

/// What the user agrees to: shown before anything is downloaded. `offerId` binds the agreement to exactly this
/// offer (single use).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InstallOffer {
    /// Random id of this offer; `confirm` must name it.
    pub offer_id: String,
    /// The Vibe version that gets installed.
    pub vibe_version: String,
    /// The Python version uv installs for it.
    pub python: String,
    /// The app-private folder everything goes into (nothing outside it, no PATH change).
    pub folder: String,
    /// The official sources, with checksums where we download a file ourselves.
    pub sources: Vec<SourceView>,
    /// The commands that run, in order (paths shortened to the install folder).
    pub steps: Vec<String>,
    /// SHA-256 of the hash-locked requirements file (every package and its hash is pinned).
    pub requirements_sha256: String,
    /// Packages installed (Vibe and its dependencies).
    pub packages: u32,
    /// Download size estimate in bytes.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub download_bytes: u64,
    /// Disk space estimate in bytes after the install.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub disk_bytes: u64,
    /// Vibe's official documentation.
    pub docs_url: String,
}

/// Why an install stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum FailKind {
    /// No connection (offline, DNS, refused, proxy unreachable).
    Network,
    /// A download did not have the pinned checksum (it was deleted, never run).
    Checksum,
    /// A step took longer than its limit.
    Timeout,
    /// The user cancelled.
    Cancelled,
    /// A step failed (its output tail says why).
    Step,
    /// Installed, but Vibe did not start.
    Start,
    /// The disk or the install folder (permissions, space).
    Disk,
}

/// Where the installed server comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum InstallSource {
    /// Installed by Vibe Work into its own folder.
    Private,
    /// The user's own install, chosen in setup.
    Existing,
}

/// The install's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum InstallPhase {
    /// Nothing installed and nothing chosen.
    NotInstalled {},
    /// An offer waits for the user's answer.
    Offered {
        /// The offer.
        offer: InstallOffer,
    },
    /// The install runs.
    #[serde(rename_all = "camelCase")]
    Installing {
        /// Name of the current step.
        step: String,
        /// 1-based number of the current step.
        index: u32,
        /// Steps in all.
        total: u32,
        /// Milliseconds since the install started.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        elapsed_ms: u64,
        /// The last lines of the current step's output.
        output_tail: String,
    },
    /// The install stopped; `retry` goes back to a fresh offer.
    #[serde(rename_all = "camelCase")]
    Failed {
        /// The step that failed.
        step: String,
        /// Why.
        kind: FailKind,
        /// Plain-words reason.
        error: String,
        /// The last lines of the step's output.
        output_tail: String,
    },
    /// Vibe is ready (installed by us, or the user's own install).
    #[serde(rename_all = "camelCase")]
    Installed {
        /// Its version.
        version: String,
        /// The `vibe-app-server` executable.
        server: String,
        /// Ours or the user's.
        source: InstallSource,
        /// Disk use of our install in bytes (`null` for the user's own).
        #[cfg_attr(feature = "ts", ts(type = "number | null"))]
        disk_bytes: Option<u64>,
        /// Install time in milliseconds (`null` when it was installed in an earlier run).
        #[cfg_attr(feature = "ts", ts(type = "number | null"))]
        install_ms: Option<u64>,
        /// A version warning (an existing install of another version), else `null`.
        warning: Option<String>,
    },
}

/// A `vibe-app-server` found on this machine (PATH or a known install location).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ExistingView {
    /// The executable (canonical).
    pub path: String,
    /// Its version from the `initialize` answer, `null` when it did not answer.
    pub version: Option<String>,
    /// Exactly the pinned version (the protocol types match).
    pub supported: bool,
    /// The version-mismatch warning shown before it can be chosen, when the version differs.
    pub warning: Option<String>,
}

/// The whole picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InstallView {
    /// The version Vibe Work is built for.
    pub pinned_version: String,
    /// The current phase.
    pub phase: InstallPhase,
    /// Existing installs found by the last check (empty until `checkExisting` ran).
    pub existing: Vec<ExistingView>,
    /// Whether `checkExisting` ran in this app run.
    pub checked_existing: bool,
    /// The older app-owned Vibe version this install replaces after an app update (setup then says Update, not
    /// Install); absent for a fresh install or the user's own install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub replacing: Option<String>,
    /// Vibe updates from the signed supported-Vibe manifest (boxed: the view travels in core events).
    pub update: Box<VibeUpdateView>,
}

/// The state of a Vibe update install. The running Vibe keeps running meanwhile; the new version
/// starts with Vibe Work's next start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum VibeUpdatePhase {
    /// No update is being installed.
    Idle {},
    /// An offer waits for the user's answer (the consent dialog shows it).
    Offered {
        /// The offer (same shape as the setup's).
        offer: InstallOffer,
    },
    /// The update installs next to the running Vibe.
    #[serde(rename_all = "camelCase")]
    Installing {
        /// Name of the current step.
        step: String,
        /// 1-based number of the current step.
        index: u32,
        /// Steps in all.
        total: u32,
        /// Milliseconds since the install started.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        elapsed_ms: u64,
        /// The last lines of the current step's output.
        output_tail: String,
    },
    /// The update install stopped; the running Vibe is untouched.
    #[serde(rename_all = "camelCase")]
    Failed {
        /// The step that failed.
        step: String,
        /// Why.
        kind: FailKind,
        /// Plain-words reason.
        error: String,
        /// The last lines of the step's output.
        output_tail: String,
    },
    /// Installed; it starts the next time Vibe Work starts.
    #[serde(rename_all = "camelCase")]
    Installed {
        /// The installed version.
        version: String,
    },
}

/// What the update area shows about Vibe, from the last check of the signed supported-Vibe manifest.
/// Every field is empty until a check ran and verified; a failed check empties the offers (fail closed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VibeUpdateView {
    /// A tested Vibe newer than the running one that Vibe Work can install ("Vibe Y available → Update").
    pub available: Option<String>,
    /// A newer Vibe on `PyPI` without support yet ("support arrives in an upcoming update"); never installed.
    pub upcoming: Option<String>,
    /// For a user's own install: the version notice (nothing is installed for it).
    pub mismatch: Option<String>,
    /// Unix seconds of the last check, `null` before the first.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub checked_at: Option<u64>,
    /// Why the last check gave no offer (no channel file, bad signature, older list), `null` when it worked.
    pub check_error: Option<String>,
    /// The update install.
    pub phase: VibeUpdatePhase,
}

impl Default for VibeUpdateView {
    fn default() -> Self {
        Self {
            available: None,
            upcoming: None,
            mismatch: None,
            checked_at: None,
            check_error: None,
            phase: VibeUpdatePhase::Idle {},
        }
    }
}
