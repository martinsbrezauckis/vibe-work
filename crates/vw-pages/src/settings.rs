//! Per-project handbook settings, kept in app data (never in the repository): a handbook folder the user chose
//! (default: none, the folder is resolved by `handbook::resolve_folder`: `docs/handbook/`, else a pre-1.0
//! `docs/pages/`) and whether prototype pages may run their script (default on: clickable mockups need it; Format 1.2
//! §6.4: pages of every other kind never run script, whatever this says).
//!
//! Depends on: serde, vw-platform, `paths`, `comments` (project key). Called by: `service`, vw-core-api.
//! Invariants: the folder is a clean relative path inside the project (`paths::clean_relative`); a settings file
//! that does not parse reads as the defaults; files are user-only and replaced atomically.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::{DEFAULT_FOLDER, clean_relative};

/// One project's settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PagesSettings {
    /// The handbook folder, relative to the project folder. In answers: the folder in use (chosen or resolved).
    pub folder: String,
    /// Prototype pages (Format 1.2, kind `prototype`) may run their own script in the viewer, sandboxed and without
    /// network. Pages of every other kind never run script.
    pub scripts: bool,
    /// The user chose `folder`; `false`: the folder is resolved (`docs/handbook`, else a pre-1.0 `docs/pages`), and
    /// a saved `folder` is ignored.
    #[serde(default)]
    pub custom: bool,
}

impl Default for PagesSettings {
    fn default() -> Self {
        Self {
            folder: DEFAULT_FOLDER.to_owned(),
            scripts: true,
            custom: false,
        }
    }
}

impl PagesSettings {
    /// The pages folder of `project`.
    #[must_use]
    pub fn pages_dir(&self, project: &Path) -> PathBuf {
        self.folder
            .split('/')
            .fold(project.to_path_buf(), |p, s| p.join(s))
    }
}

/// Where the settings of every project live.
pub struct SettingsStore {
    dir: PathBuf,
}

impl SettingsStore {
    /// A store in `dir`.
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn file(&self, project: &Path) -> PathBuf {
        self.dir
            .join(format!("{}.json", crate::comments::project_key(project)))
    }

    /// The saved settings of `project` (`None` when none were saved or the file is unreadable or invalid).
    #[must_use]
    pub fn saved(&self, project: &Path) -> Option<PagesSettings> {
        std::fs::read_to_string(self.file(project))
            .ok()
            .and_then(|t| serde_json::from_str::<PagesSettings>(&t).ok())
            .filter(|s| clean_relative(&s.folder).is_ok())
    }

    /// The settings of `project` with the folder in use: the chosen one, else the resolved one (see the module
    /// header). The second value is true when the resolved folder is the pre-1.0 `docs/pages/`.
    #[must_use]
    pub fn get(&self, project: &Path) -> (PagesSettings, bool) {
        let saved = self.saved(project);
        let chosen = saved
            .as_ref()
            .filter(|s| s.custom)
            .map(|s| s.folder.as_str());
        let folder = crate::handbook::resolve_folder(project, chosen);
        (
            PagesSettings {
                folder: folder.relative,
                scripts: saved.as_ref().is_none_or(|s| s.scripts),
                custom: chosen.is_some(),
            },
            folder.legacy,
        )
    }

    /// Saves `settings` for `project` after checking the folder.
    ///
    /// # Errors
    /// A plain-words reason (bad folder, write failure).
    pub fn set(&self, project: &Path, settings: &PagesSettings) -> Result<PagesSettings, String> {
        let folder = if settings.custom {
            clean_relative(&settings.folder).map_err(|e| {
                format!("the handbook folder must be a folder inside the project: {e}")
            })?
        } else {
            DEFAULT_FOLDER.to_owned()
        };
        let clean = PagesSettings {
            folder,
            scripts: settings.scripts,
            custom: settings.custom,
        };
        vw_platform::ensure_private_dir(&self.dir).map_err(|e| e.to_string())?;
        let path = self.file(project);
        let temp = path.with_extension("json.new");
        let _ = std::fs::remove_file(&temp);
        let bytes = serde_json::to_vec(&clean).map_err(|e| e.to_string())?;
        vw_platform::write_private_new(&temp, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &path).map_err(|e| e.to_string())?;
        Ok(clean)
    }
}
