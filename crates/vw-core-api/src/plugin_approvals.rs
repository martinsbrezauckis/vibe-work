//! User-owned plugin approvals, kept beside Vibe home in app data. Called before server launches and
//! plugin-loading requests, and by Tools controls. Depends on private atomic writes and Vibe discovery.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use vw_platform::sync::LockRecover;

use crate::error::{CoreError, ErrorCode};

pub(crate) struct PluginApprovals {
    home: PathBuf,
    file: PathBuf,
    lock: Mutex<()>,
    pub(crate) mutations: tokio::sync::Mutex<()>,
    identity: crate::plugin_identity::PluginIdentity,
}

fn error(problem: impl std::fmt::Display) -> CoreError {
    CoreError::new(
        ErrorCode::Internal,
        format!("The plugin approvals could not be applied: {problem}"),
    )
}

impl PluginApprovals {
    pub(crate) fn new(home: &Path) -> Result<Self, CoreError> {
        let data = home
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| error("the app data folder is unavailable"))?;
        Ok(Self {
            home: home.to_path_buf(),
            file: data.join("plugin-approvals.json"),
            lock: Mutex::new(()),
            mutations: tokio::sync::Mutex::new(()),
            identity: crate::plugin_identity::PluginIdentity::new(None),
        })
    }

    pub(crate) fn configured(
        supervisor: &vw_supervisor::SupervisorConfig,
        config: &crate::core::CoreConfig,
    ) -> Result<Self, CoreError> {
        let mut approvals = Self::new(&supervisor.vibe_home)?;
        approvals.identity =
            crate::plugin_identity::PluginIdentity::new(config.market.resolver_python.clone());
        Ok(approvals)
    }

    /// The pinned Vibe's Python, for other read-only helpers (`vibe_rules`).
    pub(crate) fn vibe_python(&self) -> Option<std::path::PathBuf> {
        self.identity.python()
    }

    pub(crate) fn before_launch(&self, program: &Path) -> Result<(), CoreError> {
        self.identity.set_program(program);
        // A failed move never keeps Vibe from starting: each step leaves the plugin off, and the next start retries.
        if let Err(problem) = self.migrate_off_folders() {
            tracing::warn!(%problem, "plugins: the move of turned-off plugins did not finish");
        }
        self.sweep()
    }

    /// Once per app data folder: every readable plugin an earlier version turned off by moving it to
    /// `plugins-off` goes back to `plugins`, approved, after its name is written to Vibe's `disabled_plugins`, so
    /// it stays off (Vibe's switch decides from now on). The name is written first: an interruption between the
    /// steps leaves the plugin off either way, and the next start finishes. A folder whose name another plugin
    /// in `plugins` already has, or whose place in `plugins` is taken, stays where it is.
    fn migrate_off_folders(&self) -> Result<(), CoreError> {
        let done = self.file.with_file_name("plugin-switches-migrated");
        if std::fs::symlink_metadata(&done).is_ok() {
            return Ok(());
        }
        let _held = self.lock.lock_or_recover();
        let off_root = self.home.join("plugins-off");
        let plain = std::fs::symlink_metadata(&off_root)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
        if plain {
            let mut folders = Vec::new();
            for entry in std::fs::read_dir(&off_root).map_err(error)? {
                let entry = entry.map_err(error)?;
                if entry.file_type().map_err(error)?.is_dir() {
                    folders.push(entry.path());
                }
            }
            let names = self.identity.names(&folders, &self.home)?;
            let mut approved = self.read()?;
            let config = self.home.join("config.toml");
            for (folder, name) in folders.into_iter().zip(names) {
                let (Some(name), Some(key)) = (name, folder.file_name().and_then(|n| n.to_str()))
                else {
                    continue;
                };
                let target = self.home.join("plugins").join(key);
                if approved
                    .iter()
                    .any(|(other, live)| other != key && *live == name)
                    || std::fs::symlink_metadata(&target).is_ok()
                {
                    continue;
                }
                crate::plugin_switches::add_disabled(&config, &name).map_err(error)?;
                std::fs::create_dir_all(self.home.join("plugins")).map_err(error)?;
                std::fs::rename(&folder, &target).map_err(error)?;
                approved.insert(key.to_owned(), name);
                self.write(&approved)?;
            }
        }
        vw_platform::replace_private(&done, b"1").map_err(error)
    }

    fn read(&self) -> Result<BTreeMap<String, String>, CoreError> {
        match std::fs::symlink_metadata(&self.file) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Ok(m) if m.is_file() && !m.file_type().is_symlink() => {}
            _ => return Err(error("the approval record is not a plain file")),
        }
        let bytes = std::fs::read(&self.file).map_err(error)?;
        // A damaged record grants nothing; the next explicit user action can replace it.
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    /// The caller has resolved this name through Vibe and confirmed the user action.
    pub(crate) fn set(&self, folder: &Path, name: Option<&str>) -> Result<(), CoreError> {
        let _held = self.lock.lock_or_recover();
        let Some(key) = folder.file_name().and_then(|n| n.to_str()) else {
            return Err(error("the plugin folder has no name"));
        };
        let mut approved = self.read()?;
        if let Some(name) = name {
            approved.insert(key.to_owned(), name.to_owned());
        } else {
            approved.remove(key);
        }
        self.write(&approved)
    }

    fn write(&self, approved: &BTreeMap<String, String>) -> Result<(), CoreError> {
        std::fs::create_dir_all(
            self.file
                .parent()
                .ok_or_else(|| error("no app data folder"))?,
        )
        .map_err(error)?;
        vw_platform::replace_private(&self.file, &serde_json::to_vec(approved).map_err(error)?)
            .map_err(error)
    }

    /// Publish a completed install and its approval without a sweep between the two filesystem changes.
    pub(crate) fn commit_install(
        &self,
        store: &vw_market::Store,
        id: &str,
        staging: &Path,
    ) -> Result<(), vw_market::MarketError> {
        let _held = self.lock.lock_or_recover();
        let record_error = |e: CoreError| {
            vw_market::MarketError::io(
                "record plugin approval",
                std::io::Error::other(e.to_string()),
            )
        };
        // The same pinned resolver and bounds used by Tools must accept the staged package
        // before it is published or reported installed.
        let name = self
            .identity
            .name(staging, &self.home)
            .map_err(record_error)?
            .ok_or_else(|| {
                vw_market::MarketError::NotInstallable(
                    "This package is not readable by Vibe; nothing was installed.".to_owned(),
                )
            })?;
        let mut approved = self.read().map_err(record_error)?;
        store.commit(id, staging)?;
        approved.insert(id.to_owned(), name);
        if let Err(problem) = self.write(&approved) {
            let _ = crate::market_controls::move_plugin(&store.plugins_dir().join(id), false);
            return Err(record_error(problem));
        }
        Ok(())
    }

    /// Hold the real sweep lock while a concurrency test observes unrelated command progress.
    #[cfg(test)]
    pub(crate) fn hold_sweep(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap()
    }

    /// Move every unapproved child, including invalid or linked plugin folders, before Vibe sees the root.
    /// Any failed move stops the loading request; existing destinations are never overwritten.
    pub(crate) fn sweep(&self) -> Result<(), CoreError> {
        self.sweep_kept().map(drop)
    }

    /// [`Self::sweep`], returning the names of the folders it left in `plugins/` (all approved). A folder that
    /// appears after this call is not in the list: readers use it to skip folders no sweep has checked yet.
    pub(crate) fn sweep_kept(&self) -> Result<Vec<String>, CoreError> {
        let _held = self.lock.lock_or_recover();
        let root = self.home.join("plugins");
        let mut kept = Vec::new();
        match std::fs::symlink_metadata(&root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(kept),
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
            _ => return Err(error("the plugins root is not a plain folder")),
        }
        let approved = self.read()?;
        let mut candidates = Vec::new();
        for entry in std::fs::read_dir(&root).map_err(error)? {
            let entry = entry.map_err(error)?;
            let kind = entry.file_type().map_err(error)?;
            if !kind.is_dir() && !kind.is_symlink() {
                continue;
            }
            let folder = entry.path();
            let expected = entry.file_name().to_str().and_then(|key| approved.get(key));
            if !kind.is_symlink()
                && let Some(name) = expected
            {
                candidates.push((folder, name));
            } else {
                crate::market_controls::move_plugin(&folder, false)?;
            }
        }
        let folders: Vec<PathBuf> = candidates.iter().map(|(path, _)| path.clone()).collect();
        let names = self.identity.names(&folders, &self.home)?;
        for ((folder, expected), actual) in candidates.into_iter().zip(names) {
            if actual.as_ref() == Some(expected) {
                kept.extend(
                    folder
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_owned),
                );
            } else {
                crate::market_controls::move_plugin(&folder, false)?;
            }
        }
        Ok(kept)
    }
}
