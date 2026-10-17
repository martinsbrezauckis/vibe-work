//! User-owned plugin approvals, kept beside Vibe home in app data. Called before server launches and
//! plugin-loading requests, and by Tools controls. Depends on private atomic writes and Vibe discovery.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

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

    pub(crate) fn before_launch(&self, program: &Path) -> Result<(), CoreError> {
        self.identity.set_program(program);
        self.sweep()
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
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
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
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let record_error = |e: CoreError| {
            vw_market::MarketError::io(
                "record plugin approval",
                std::io::Error::other(e.to_string()),
            )
        };
        let mut approved = self.read().map_err(record_error)?;
        store.commit(id, staging)?;
        approved.insert(id.to_owned(), id.to_owned());
        if let Err(problem) = self.write(&approved) {
            let _ = crate::market_controls::move_plugin(&store.plugins_dir().join(id), false);
            return Err(record_error(problem));
        }
        Ok(())
    }

    /// Move every unapproved child, including invalid or linked plugin folders, before Vibe sees the root.
    /// Any failed move stops the loading request; existing destinations are never overwritten.
    pub(crate) fn sweep(&self) -> Result<(), CoreError> {
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let root = self.home.join("plugins");
        match std::fs::symlink_metadata(&root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
            _ => return Err(error("the plugins root is not a plain folder")),
        }
        let approved = self.read()?;
        for entry in std::fs::read_dir(&root).map_err(error)? {
            let entry = entry.map_err(error)?;
            let kind = entry.file_type().map_err(error)?;
            if !kind.is_dir() && !kind.is_symlink() {
                continue;
            }
            let folder = entry.path();
            let expected = entry.file_name().to_str().and_then(|key| approved.get(key));
            let allowed = if kind.is_symlink() {
                false
            } else if let Some(name) = expected {
                self.identity.name(&folder, &self.home)?.as_ref() == Some(name)
            } else {
                false
            };
            if !allowed {
                crate::market_controls::move_plugin(&folder, false)?;
            }
        }
        Ok(())
    }
}
