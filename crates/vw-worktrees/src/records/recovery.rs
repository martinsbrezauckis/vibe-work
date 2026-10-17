//! Retains ownership metadata after a failed start record write, in a separate private folder and
//! shared memory. Depends on Records and atomic JSON writes; called by the core's failed-start path
//! and record readers/updates. Recovery never deletes a session, branch or worktree.

use std::path::PathBuf;

use super::{Record, Records, WorktreeError, lock_records};

impl Records {
    fn recovery_dir(&self) -> PathBuf {
        self.dir.with_extension("recovery")
    }

    pub(super) fn recovery_file(&self, session_id: &str) -> Option<PathBuf> {
        vw_changes::valid_session_id(session_id)
            .then(|| self.recovery_dir().join(format!("{session_id}.json")))
    }

    pub(super) fn memory_recovery(&self, session_id: &str) -> Option<Record> {
        self.recovery
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session_id)
            .cloned()
    }

    /// True when a failed creation's ownership is retained in recovery storage or this run's memory.
    #[must_use]
    pub fn is_recovery(&self, session_id: &str) -> bool {
        self.memory_recovery(session_id).is_some()
            || self
                .recovery_file(session_id)
                .is_some_and(|path| path.exists())
    }

    /// Retains a failed creation's real session and branch for the leftover inventory and controls.
    /// Cloned handles share the record even when durable storage is unavailable.
    ///
    /// # Errors
    /// `InvalidName` for an unsafe session ID; `Io` when persistence fails (the valid record remains
    /// in memory, and the caller must explain that recovery is only available for this run).
    pub fn recover(&self, record: &Record) -> Result<(), WorktreeError> {
        let _guard = lock_records();
        self.recover_unlocked(record)
    }

    pub(super) fn recover_unlocked(&self, record: &Record) -> Result<(), WorktreeError> {
        let path = self.recovery_file(&record.session_id).ok_or_else(|| {
            WorktreeError::InvalidName("the session id cannot name a recovery record".into())
        })?;
        self.recovery
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(record.session_id.clone(), record.clone());
        let dir = self.recovery_dir();
        vw_platform::ensure_private_dir(&dir).map_err(|error| {
            WorktreeError::io(
                "prepare recovery records",
                &dir,
                std::io::Error::other(error.to_string()),
            )
        })?;
        let bytes = serde_json::to_vec_pretty(record).map_err(|error| {
            WorktreeError::io(
                "encode recovery record",
                &path,
                std::io::Error::other(error),
            )
        })?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)
            .map_err(|error| WorktreeError::io("write recovery record", &tmp, error))?;
        std::fs::rename(&tmp, &path)
            .map_err(|error| WorktreeError::io("replace recovery record", &path, error))?;
        // Durable records are read from disk so another handle's archive/keep/handoff remains visible.
        self.recovery
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&record.session_id);
        Ok(())
    }

    pub(super) fn recovery_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self
            .recovery
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        if let Ok(entries) = std::fs::read_dir(self.recovery_dir()) {
            ids.extend(entries.flatten().filter_map(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .strip_suffix(".json")
                    .map(str::to_owned)
            }));
        }
        ids
    }

    pub(super) fn remove_recovery(&self, id: &str) -> Result<(), WorktreeError> {
        let _guard = lock_records();
        if let Some(path) = self.recovery_file(id)
            && let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(WorktreeError::io("remove recovery record", &path, error));
        }
        self.recovery
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
        Ok(())
    }

    pub(super) fn add_recoveries_for_guard(
        &self,
        out: &mut Vec<Record>,
    ) -> Result<(), WorktreeError> {
        let dir = self.recovery_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => Some(entries),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(WorktreeError::io("read recovery records", &dir, error)),
        };
        if let Some(entries) = entries {
            for entry in entries {
                let path = entry
                    .map_err(|error| WorktreeError::io("read recovery entry", &dir, error))?
                    .path();
                let Some(id) = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.strip_suffix(".json"))
                else {
                    continue;
                };
                if !vw_changes::valid_session_id(id) {
                    continue;
                }
                let bytes = std::fs::read(&path)
                    .map_err(|error| WorktreeError::io("read recovery record", &path, error))?;
                let record: Record = serde_json::from_slice(&bytes).map_err(|error| {
                    WorktreeError::io("parse recovery record", &path, std::io::Error::other(error))
                })?;
                if record.session_id != id {
                    return Err(WorktreeError::InvalidName(
                        "recovery record names a different session".into(),
                    ));
                }
                out.retain(|existing| existing.session_id != id);
                out.push(self.follow_rename(record)?);
            }
        }
        let memory = self
            .recovery
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for (id, record) in memory {
            out.retain(|existing| existing.session_id != id);
            out.push(self.follow_rename(record)?);
        }
        Ok(())
    }
}
