//! Redaction of grouped findings, guarded by existing channels and session leases.
//! Depends on live scanner reports and the worker cache; called by the worker. Replacements are rescanned.

use super::{Worker, count, failure, finding_id, hits, inputs, inside_log_folder, owned};
use crate::{
    Core, CoreError, ErrorCode, SecretLeakName, SecretLeakState,
    core::Shared,
    secret_scan,
    secret_scan_cache::{Stamp, fingerprint},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{PoisonError, atomic::Ordering},
};
use vw_keys::VaultScanSecret;

#[derive(Default)]
struct FileOutcome {
    redacted: u32,
    changed: u32,
    format: u32,
    unreadable: u32,
    names: BTreeMap<String, SecretLeakName>,
}

impl Worker {
    pub(super) fn redact(
        &mut self,
        shared: &std::sync::Arc<Shared>,
        runtime: &tokio::runtime::Handle,
        id: &str,
    ) -> Result<(), CoreError> {
        let secrets = match inputs(shared) {
            Ok(secrets) => {
                self.restrict_inputs(shared, &secrets);
                Some(secrets)
            }
            // Live reports were admitted on this worker; locking does not add comparison authority.
            Err(error) if error.code == ErrorCode::KeyService => None,
            Err(error) => return Err(error),
        };
        let finding = self
            .cache
            .findings
            .iter()
            .find(|finding| finding.finding_id == id)
            .cloned()
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::NotFound,
                    "The session log finding is no longer available.",
                )
            })?;
        if finding.state == SecretLeakState::Removed {
            return Ok(());
        }
        let core = Core {
            shared: shared.clone(),
        };
        // The existing lease arbitrates app windows. Vibe's native holder is checked without opening its channel.
        if shared.channels.has_channel(&finding.chat_id)
            || runtime
                .block_on(core.session_holder(&finding.chat_id))?
                .is_some()
        {
            self.set_state(shared, id, SecretLeakState::ChatOpen, 0);
            return Ok(());
        }
        let _lease = match shared.leases.claim(&finding.chat_id) {
            Ok(lease) => lease,
            Err(error) if error.server_code.as_deref() == Some("sessionHeld") => {
                self.set_state(shared, id, SecretLeakState::ChatOpen, 0);
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let paths: Vec<_> = self
            .cache
            .files
            .iter()
            .filter(|(_, file)| file.chat_id == finding.chat_id && !file.hits.is_empty())
            .map(|(path, _)| path.clone())
            .collect();
        self.ensure_scope(shared, runtime, id, &paths)?;
        let mut outcome = FileOutcome::default();
        for path in paths {
            if shared.secret_scan.stopped.load(Ordering::Acquire) {
                return Ok(());
            }
            let file = self.redact_file(&path, secrets.as_deref())?;
            outcome.redacted += file.redacted;
            outcome.changed += file.changed;
            outcome.format += file.format;
            outcome.unreadable += file.unreadable;
            for (id, name) in file.names {
                outcome
                    .names
                    .entry(id)
                    .and_modify(|old| old.places += name.places)
                    .or_insert(name);
            }
        }
        if secrets.is_none() {
            self.pending_rescans.insert(finding.chat_id.clone());
        }
        self.finish_redaction(shared, id, &finding, outcome);
        Ok(())
    }

    fn finish_redaction(
        &mut self,
        shared: &Shared,
        id: &str,
        finding: &crate::SecretLeakFinding,
        outcome: FileOutcome,
    ) {
        let skipped = outcome.changed + outcome.format + outcome.unreadable > 0;
        self.refresh(shared);
        self.record_removed(id, outcome.names, skipped.then_some(finding.files));
        let remaining = self
            .cache
            .files
            .values()
            .filter(|file| file.chat_id == finding.chat_id)
            .any(|file| !file.hits.is_empty());
        let state = if skipped {
            SecretLeakState::Partial
        } else if remaining {
            SecretLeakState::Found
        } else {
            SecretLeakState::Removed
        };
        self.set_state(shared, id, state, outcome.redacted);
        if let Some(finding) = self
            .cache
            .findings
            .iter_mut()
            .find(|finding| finding.finding_id == id)
        {
            finding.changed_files = outcome.changed;
            finding.format_files = outcome.format;
            finding.unreadable_files = outcome.unreadable;
        }
        shared
            .secret_scan
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .findings
            .clone_from(&self.cache.findings);
        secret_scan::publish(shared);
    }

    fn ensure_scope(
        &mut self,
        shared: &std::sync::Arc<Shared>,
        runtime: &tokio::runtime::Handle,
        id: &str,
        paths: &[PathBuf],
    ) -> Result<(), CoreError> {
        if paths.is_empty() {
            return Err(failure(
                "Check this chat again before redacting the saved finding.",
            ));
        }
        if paths.iter().all(|path| {
            self.reports
                .get(id)
                .is_some_and(|reports| reports.contains_key(path))
        }) {
            return Ok(());
        }
        let folder = if let Some(folder) = &self.folder {
            folder.clone()
        } else {
            runtime.block_on(secret_scan::resolve_folder(&Core {
                shared: shared.clone(),
            }))?
        };
        for path in paths {
            if !inside_log_folder(&folder, path) {
                return Err(failure(
                    "The saved finding is outside the configured session log folder.",
                ));
            }
        }
        self.folder = Some(folder);
        Ok(())
    }

    fn record_removed(
        &mut self,
        id: &str,
        removed: BTreeMap<String, SecretLeakName>,
        original_files: Option<u32>,
    ) {
        let Some(finding) = self
            .cache
            .findings
            .iter_mut()
            .find(|finding| finding.finding_id == id)
        else {
            return;
        };
        if let Some(files) = original_files {
            finding.files = files;
        }
        if !finding
            .secrets
            .iter()
            .any(|name| name.state == SecretLeakState::Found)
        {
            return;
        }
        for name in removed.into_values() {
            if let Some(old) = finding
                .secrets
                .iter_mut()
                .find(|old| old.name == name.name && old.website_password == name.website_password)
            {
                if old.state == SecretLeakState::Found {
                    old.state = SecretLeakState::Partial;
                    old.places += name.places;
                } else {
                    *old = name;
                }
            } else {
                finding.secrets.push(name);
            }
        }
    }

    /// A Vibe 3.0 store is sealed by digests: a replaced value would make the chat unloadable, so its files are
    /// reported as "could not be changed without damaging the chat" and left exactly as they are. Without a known
    /// log folder a file counts as sealed.
    pub(crate) fn sealed_file(&self, path: &std::path::Path) -> bool {
        self.folder
            .as_deref()
            .is_none_or(|folder| crate::secret_scan_cache::sealed(folder, path))
    }

    fn redact_file(
        &mut self,
        path: &PathBuf,
        secrets: Option<&[VaultScanSecret]>,
    ) -> Result<FileOutcome, CoreError> {
        if self.sealed_file(path) {
            return Ok(FileOutcome {
                format: 1,
                ..FileOutcome::default()
            });
        }
        self.redact_unsealed_file(path, secrets)
    }

    fn redact_unsealed_file(
        &mut self,
        path: &PathBuf,
        secrets: Option<&[VaultScanSecret]>,
    ) -> Result<FileOutcome, CoreError> {
        let authority = finding_id(&self.cache.files[path].chat_id);
        let reports = self.reports.entry(authority).or_default();
        let mut changed = false;
        if !reports.contains_key(path) {
            let secrets = secrets
                .ok_or_else(|| failure("Unlock the Vault to check this saved finding again."))?;
            let stamp =
                Stamp::read(path).map_err(|_| failure("Could not recheck the saved finding."))?;
            changed = stamp != self.cache.files[path].stamp
                || fingerprint(secrets) != self.cache.files[path].fingerprint;
            reports.insert(
                path.clone(),
                vw_logscan::scan_files(vec![path.clone()], owned(secrets))
                    .map_err(|_| failure("Could not recheck the session log."))?,
            );
        }
        if changed {
            if let Some(file) = self.cache.files.get_mut(path) {
                if let Some(secrets) = secrets {
                    file.hits = hits(&reports[path], secrets);
                    file.fingerprint = fingerprint(secrets);
                }
                file.places = count(reports[path].place_count());
                if let Ok(stamp) = Stamp::read(path) {
                    file.stamp = stamp;
                }
            }
            return Ok(FileOutcome {
                changed: 1,
                ..FileOutcome::default()
            });
        }
        let report = &reports[path];
        let file = &self.cache.files[path];
        let ids: BTreeSet<String> = file.hits.keys().cloned().collect();
        if ids.is_empty() {
            return Ok(FileOutcome::default());
        }
        let selected = report
            .select_secrets(&ids)
            .with_format_guard(crate::secret_scan_format::safe);
        let result = vw_logscan::redact(&selected);
        let replaced = count(result.redacted_files.len());
        let removed = if replaced > 0 {
            file.hits
                .iter()
                .filter(|(id, _)| ids.contains(*id))
                .map(|(id, name)| {
                    (
                        id.clone(),
                        SecretLeakName {
                            state: SecretLeakState::Removed,
                            ..name.clone()
                        },
                    )
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        // A successful replacement invalidates the old authority immediately.
        reports.remove(path);
        if let Some(secrets) = secrets {
            let report = vw_logscan::scan_files(vec![path.clone()], owned(secrets))
                .map_err(|_| failure("Could not recheck the redacted chat."))?;
            let remaining = hits(&report, secrets);
            if let Some(file) = self.cache.files.get_mut(path) {
                file.hits = remaining;
                file.places = count(report.place_count());
                if let Ok(stamp) = Stamp::read(path) {
                    file.stamp = stamp;
                }
            }
            reports.insert(path.clone(), report);
        } else if result.skipped_files.is_empty()
            && let Some(file) = self.cache.files.get_mut(path)
        {
            file.hits.clear();
            file.places = 0;
        }
        let mut outcome = FileOutcome {
            redacted: replaced,
            names: removed,
            ..FileOutcome::default()
        };
        for issue in &result.skipped_files {
            match issue.reason {
                vw_logscan::FileSkipReason::ChangedSinceScan => outcome.changed += 1,
                vw_logscan::FileSkipReason::InvalidJson => outcome.format += 1,
                vw_logscan::FileSkipReason::Io => outcome.unreadable += 1,
            }
        }
        Ok(outcome)
    }

    fn set_state(&mut self, shared: &Shared, id: &str, state: SecretLeakState, files: u32) {
        if let Some(finding) = self
            .cache
            .findings
            .iter_mut()
            .find(|finding| finding.finding_id == id)
        {
            finding.state = state;
            finding.redacted_files = files;
            if state == SecretLeakState::Removed {
                for name in &mut finding.secrets {
                    name.state = state;
                }
            }
        }
        shared
            .secret_scan
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .findings
            .clone_from(&self.cache.findings);
        secret_scan::publish(shared);
    }

    /// Complete the explicit manual action's rescan after an unlock, even when scanning was switched off.
    pub(super) fn recheck_pending(&mut self, shared: &Shared) -> Result<(), CoreError> {
        let secrets = match inputs(shared) {
            Ok(secrets) => secrets,
            Err(error) if error.code == ErrorCode::KeyService => return Ok(()),
            Err(error) => return Err(error),
        };
        self.restrict_inputs(shared, &secrets);
        let fingerprint = fingerprint(&secrets);
        for id in std::mem::take(&mut self.pending_rescans) {
            let paths: Vec<_> = self
                .cache
                .files
                .iter()
                .filter(|(_, file)| file.chat_id == id)
                .map(|(path, _)| path.clone())
                .collect();
            for path in paths {
                let report = vw_logscan::scan_files(vec![path.clone()], owned(&secrets))
                    .map_err(|_| failure("Could not recheck the redacted chat."))?;
                if let Some(file) = self.cache.files.get_mut(&path) {
                    file.hits = hits(&report, &secrets);
                    file.places = count(report.place_count());
                    file.fingerprint.clone_from(&fingerprint);
                    if let Ok(stamp) = Stamp::read(&path) {
                        file.stamp = stamp;
                    }
                }
                self.reports
                    .entry(finding_id(&id))
                    .or_default()
                    .insert(path, report);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        self.refresh(shared);
        secret_scan::status(shared, crate::SecretScanStatus::Checked);
        Ok(())
    }
}
