//! Shared search/document/upload exclusion for app state and every known vault location.
//! Uses the folder resolver and opened-file paths; callers check before reading any bytes.
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vw_agent_tools::folders::{Layout, resolve};

use crate::core::Shared;

const PROTECTED: &str =
    "Not run: this file belongs to the vault or app state. Choose a file outside those folders.";
const MAX_IDENTITY_ENTRIES: usize = 4096;
const IDENTITY_BUDGET: Duration = Duration::from_millis(100);

pub(crate) struct ReadExclusions {
    #[cfg(test)]
    clock: Option<std::sync::Arc<dyn Fn() -> Instant + Send + Sync>>,
    data: Option<PathBuf>,
    own: Option<PathBuf>,
    vaults: Vec<PathBuf>,
}

pub(crate) fn under(path: &Path, root: &Path) -> bool {
    vw_procmon::normalize(path).starts_with(vw_procmon::normalize(root))
}

impl ReadExclusions {
    pub(crate) fn for_chat(shared: &Shared, cwd: &Path) -> Result<Self, String> {
        Self::new(
            &shared.instructions.own_home,
            cwd,
            shared.config.keys.as_ref().map(|k| k.vault_profile_dir()),
            &vw_vault::location::ProfileInput::from_env(),
        )
    }

    pub(crate) fn new(
        home: &Path,
        cwd: &Path,
        selected: Option<PathBuf>,
        input: &vw_vault::location::ProfileInput,
    ) -> Result<Self, String> {
        let layout = Layout::new(home, cwd).map_err(|e| e.to_string())?;
        let data = home
            .parent()
            .map(resolve)
            .transpose()
            .map_err(|e| e.to_string())?;
        let mut paths = Vec::new();
        paths.extend(selected);
        if let Some(data) = &data {
            paths.push(
                vw_vault::private_location::private_vault_dir(data).map_err(|e| e.to_string())?,
            );
        }
        if let Ok(shared) = vw_vault::location::resolve(input) {
            paths.extend([shared.canonical, shared.legacy]);
        }
        let vaults = paths
            .iter()
            .map(|p| resolve(p).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            #[cfg(test)]
            clock: None,
            data,
            own: layout.own,
            vaults,
        })
    }

    pub(crate) fn excludes(&self, path: &Path) -> bool {
        self.vaults.iter().any(|v| under(path, v))
            || self.data.as_ref().is_some_and(|d| under(path, d))
                && !self.own.as_ref().is_some_and(|c| under(path, c))
    }

    /// Validate the opened object, not just the spelling checked before opening it. Refuse a replacement
    /// between resolution and open; the same handle supplies all bytes after this check.
    pub(crate) fn check_opened(
        &self,
        file: &File,
        named: &Path,
        deadline: Option<Instant>,
    ) -> Result<(), String> {
        let changed =
            || "The file's location could not be confirmed. Choose the file again.".to_owned();
        let opened = vw_platform::final_path::final_path(file).map_err(|_| changed())?;
        if self.excludes(&opened) || self.excludes(named) {
            return Err(PROTECTED.into());
        }
        let real = named.canonicalize().map_err(|_| changed())?;
        if opened != real {
            return Err(changed());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let at = std::fs::metadata(&real).map_err(|_| changed())?;
            let actual = file.metadata().map_err(|_| changed())?;
            if at.dev() != actual.dev() || at.ino() != actual.ino() {
                return Err(changed());
            }
        }
        let (identity, links) =
            vw_platform::final_path::identity(file).map_err(|_| PROTECTED.to_owned())?;
        if links > 1 {
            self.check_hard_links(identity, deadline)
                .map_err(|()| PROTECTED.to_owned())?;
        }
        Ok(())
    }

    // Only multiply linked files need the protected-location walk. Missing roots contain no files;
    // an unreadable entry or identity cannot establish that this alias is outside the vault/app state.
    fn check_hard_links(
        &self,
        identity: vw_platform::final_path::FileIdentity,
        deadline: Option<Instant>,
    ) -> Result<(), ()> {
        let limit = self.now() + IDENTITY_BUDGET;
        let deadline = deadline.map_or(limit, |deadline| deadline.min(limit));
        let mut visited = 0;
        for root in self.data.iter().chain(&self.vaults) {
            if self.now() >= deadline {
                return Err(());
            }
            match std::fs::symlink_metadata(root) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(()),
                Ok(_) => {}
            }
            let mut entries = walkdir::WalkDir::new(root)
                .follow_links(false)
                .follow_root_links(false)
                // Do not buffer an unbounded directory when the walk runs out of open handles.
                .max_open(MAX_IDENTITY_ENTRIES)
                .into_iter();
            loop {
                if visited >= MAX_IDENTITY_ENTRIES || self.now() >= deadline {
                    return Err(());
                }
                let Some(entry) = entries.next() else { break };
                visited += 1;
                let entry = entry.map_err(|_| ())?;
                if !self.excludes(entry.path()) {
                    if entry.file_type().is_dir() {
                        entries.skip_current_dir();
                    }
                    continue;
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                if self.now() >= deadline {
                    return Err(());
                }
                let file = File::open(entry.path()).map_err(|_| ())?;
                let (protected, _) = vw_platform::final_path::identity(&file).map_err(|_| ())?;
                if protected == identity {
                    return Err(());
                }
            }
        }
        if self.now() >= deadline {
            Err(())
        } else {
            Ok(())
        }
    }

    #[cfg_attr(
        not(test),
        expect(clippy::unused_self, reason = "tests inject a per-policy clock")
    )]
    fn now(&self) -> Instant {
        #[cfg(test)]
        if let Some(clock) = &self.clock {
            return clock();
        }
        Instant::now()
    }
}

#[cfg(test)]
#[path = "read_exclusions_tests.rs"]
pub(crate) mod tests;
