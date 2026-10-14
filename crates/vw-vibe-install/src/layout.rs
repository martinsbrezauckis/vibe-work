//! Where the app-private Vibe lives and how a finished install is recorded.
//!
//! ```text
//! <root>/                       the app's data folder + `vibe/` (user-only, like the rest of it)
//!   <version>/                  one install per pinned Vibe version
//!     uv/uv(.exe)               Astral's uv, checksum-verified
//!     python/                   the Python uv installed (UV_PYTHON_INSTALL_DIR; no PATH link, no registry)
//!     venv/                     mistral-vibe and its pinned dependencies
//!     requirements.txt          the hash-locked requirements that were installed
//!     installed.json            written LAST: the install is complete only when it exists and matches
//!     dl/, cache/               download and uv cache, deleted after a successful install
//!   choice.json                 "use my existing install": the path the user chose (and its version)
//!   active.json                 an installed Vibe update: its version wins over the pinned one at start
//!   support-seen.json           the highest supported-Vibe manifest sequence accepted (rollback guard)
//!   probe/                      throwaway homes of the existing-install probes
//! ```
//! A venv cannot be moved (its scripts name their Python by absolute path), so the install is done in place and
//! `installed.json` is the commit point; a folder without it is an unfinished install and is removed before the
//! next attempt.
//!
//! Depends on: serde, `serde_json`. Called by: [`crate::manager`], the app (via the manager), tests.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The install record (`installed.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    /// The Vibe version installed.
    pub version: String,
    /// The `vibe-app-server` executable.
    pub server: PathBuf,
    /// uv version used.
    pub uv: String,
    /// SHA-256 of the requirements file installed.
    pub requirements_sha256: String,
    /// Disk use after the install, in bytes.
    pub disk_bytes: u64,
    /// How long the install took, in milliseconds.
    pub install_ms: u64,
    /// When it finished (seconds since the Unix epoch).
    pub finished_at: u64,
}

/// The user's choice of an existing install (`choice.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    /// The `vibe-app-server` the user chose (canonical).
    pub path: PathBuf,
    /// Its version when it was chosen.
    pub version: String,
}

/// Paths under one root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// `<app data>/vibe`.
    pub root: PathBuf,
    /// The pinned version (a folder name: digits and dots only, checked by [`Layout::new`]).
    pub version: String,
}

impl Layout {
    /// A layout, or `None` when `version` is not a plain dotted version (it becomes a folder name).
    #[must_use]
    pub fn new(root: PathBuf, version: &str) -> Option<Self> {
        let plain = !version.is_empty()
            && version
                .split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        plain.then(|| Self {
            root,
            version: version.to_owned(),
        })
    }

    /// `<root>/<version>`.
    #[must_use]
    pub fn install_dir(&self) -> PathBuf {
        self.root.join(&self.version)
    }

    /// The uv executable.
    #[must_use]
    pub fn uv_dir(&self) -> PathBuf {
        self.install_dir().join("uv")
    }

    /// The venv.
    #[must_use]
    pub fn venv(&self) -> PathBuf {
        self.install_dir().join("venv")
    }

    /// The venv's Python.
    #[must_use]
    pub fn venv_python(&self) -> PathBuf {
        if cfg!(windows) {
            self.venv().join("Scripts").join("python.exe")
        } else {
            self.venv().join("bin").join("python")
        }
    }

    /// The installed server executable.
    #[must_use]
    pub fn server(&self) -> PathBuf {
        let bin = if cfg!(windows) { "Scripts" } else { "bin" };
        self.venv().join(bin).join(vw_supervisor::BINARY_NAME)
    }

    /// `installed.json`.
    #[must_use]
    pub fn marker(&self) -> PathBuf {
        self.install_dir().join("installed.json")
    }

    /// `choice.json`.
    #[must_use]
    pub fn choice_file(&self) -> PathBuf {
        self.root.join("choice.json")
    }

    /// The probe homes' folder.
    #[must_use]
    pub fn probe_dir(&self) -> PathBuf {
        self.root.join("probe")
    }

    /// The finished install, if `installed.json` exists, names this version and its server exists.
    #[must_use]
    pub fn installed(&self) -> Option<Installed> {
        let text = std::fs::read_to_string(self.marker()).ok()?;
        let record: Installed = serde_json::from_str(&text).ok()?;
        (record.version == self.version
            && record.server == self.server()
            && record.server.is_file())
        .then_some(record)
    }

    /// The user's choice, if `choice.json` exists and its server is still an executable file.
    #[must_use]
    pub fn choice(&self) -> Option<Choice> {
        let text = std::fs::read_to_string(self.choice_file()).ok()?;
        let choice: Choice = serde_json::from_str(&text).ok()?;
        vw_supervisor::canonical_executable(&choice.path).map(|path| Choice { path, ..choice })
    }

    /// Writes `choice.json`.
    ///
    /// # Errors
    /// The OS error.
    pub fn save_choice(&self, choice: &Choice) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let text = serde_json::to_string_pretty(choice).map_err(std::io::Error::other)?;
        std::fs::write(self.choice_file(), text)
    }

    /// Removes `choice.json` (installing our own replaces a chosen existing install).
    pub fn clear_choice(&self) {
        let _ = std::fs::remove_file(self.choice_file());
    }

    /// Removes an unfinished install folder (no `installed.json`) before a new attempt, or after a failed one.
    /// Only `<root>/<version>` is ever removed (the version is digits and dots).
    ///
    /// # Errors
    /// The OS error.
    pub fn remove_unfinished(&self) -> std::io::Result<()> {
        let dir = self.install_dir();
        if dir.exists() && self.installed().is_none() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }
}

/// The version a finished Vibe update made current (`<root>/active.json`), if any. Only a plain
/// dotted version is returned (it is a folder name).
#[must_use]
pub fn active_version(root: &Path) -> Option<String> {
    #[derive(Deserialize)]
    struct Active {
        version: String,
    }
    let text = std::fs::read_to_string(root.join("active.json")).ok()?;
    let active: Active = serde_json::from_str(&text).ok()?;
    Layout::new(root.to_path_buf(), &active.version).map(|_| active.version)
}

/// Records `version` as the current Vibe (written after its `installed.json`: the switch's commit point; the next
/// start uses it). Temp file, then rename.
///
/// # Errors
/// The OS error.
pub fn save_active(root: &Path, version: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let temp = root.join("active.json.tmp");
    let text = serde_json::json!({ "version": version }).to_string();
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, root.join("active.json"))
}

/// Removes every other install folder under `root` (folders named like a plain version, except `keep`): the
/// Vibe an update replaced, once the new one is the one starting. Nothing else under `root` is touched.
pub fn remove_other_versions(root: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if is_dir && name != keep && Layout::new(root.to_path_buf(), &name).is_some() {
            match std::fs::remove_dir_all(entry.path()) {
                Ok(()) => tracing::info!(version = %name, "replaced Vibe install removed"),
                Err(error) => {
                    tracing::warn!(version = %name, %error, "replaced Vibe install not removed");
                }
            }
        }
    }
}

/// Total size of the regular files under `dir` (links not followed).
#[must_use]
pub fn tree_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => tree_bytes(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

/// Whether a `<name>-<version>.dist-info` folder exists under `venv` (site-packages is at most 4 levels down:
/// `lib/python3.12/site-packages` or `Lib/site-packages`).
#[must_use]
pub fn has_dist_info(venv: &Path, name: &str, version: &str) -> bool {
    fn walk(dir: &Path, wanted: &str, depth: u32) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            path.is_dir()
                && (entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(wanted)
                    || (depth > 0 && walk(&path, wanted, depth - 1)))
        })
    }
    walk(venv, &format!("{name}-{version}.dist-info"), 4)
}
