//! One background thread owns scanning, incremental results and live redaction reports.
//! Depends on vw-logscan, Vault inputs and session leases; called by `secret_scan`. Only names/counts/states are published.

use crate::{
    CoreError, ErrorCode, SecretLeakFinding, SecretLeakName, SecretLeakState, SecretScanStatus,
};
use crate::{
    core::Shared,
    secret_scan::{self, Job},
    secret_scan_cache::{Cache, CachedFile, Stamp, chat, clean, fingerprint, withhold_parts},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{PoisonError, Weak, atomic::Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use vw_keys::VaultScanSecret;
use vw_logscan::ScanReport;

#[path = "secret_scan_policy.rs"]
mod policy;
#[path = "secret_scan_redact.rs"]
mod redact;

struct ChatHits {
    title: String,
    secrets: BTreeMap<String, SecretLeakName>,
    files: u32,
    places: u32,
    /// One of its files is not sealed (a log from before Vibe 3.0), so Redact can work there.
    redactable: bool,
}

struct Worker {
    cache: Cache,
    reports: BTreeMap<String, BTreeMap<PathBuf, ScanReport>>,
    folder: Option<PathBuf>,
    pending_rescans: BTreeSet<String>,
}

pub(crate) fn run(
    shared: &Weak<Shared>,
    runtime: &tokio::runtime::Handle,
    receiver: std::sync::mpsc::Receiver<Job>,
) {
    if vw_platform::lower_thread_priority().is_err() {
        if let Some(shared) = shared.upgrade() {
            secret_scan::status(&shared, SecretScanStatus::Error);
        }
        return;
    }
    let Some(initial) = shared.upgrade() else {
        return;
    };
    let Some(data) = initial.config.secret_scan_data.as_ref() else {
        return;
    };
    let cache_path = data.join("secret-scan.json");
    let mut worker = Worker {
        cache: Cache::load(&cache_path),
        reports: BTreeMap::new(),
        folder: None,
        pending_rescans: BTreeSet::new(),
    };
    initial
        .secret_scan
        .view
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .findings
        .clone_from(&worker.cache.findings);
    secret_scan::publish(&initial);
    drop(initial);
    for job in receiver {
        if matches!(job, Job::Stop) {
            break;
        }
        let Some(shared) = shared.upgrade() else {
            break;
        };
        if shared.secret_scan.stopped.load(Ordering::Acquire) {
            break;
        }
        match job {
            Job::VaultChanged => {
                if worker.pending_rescans.is_empty() {
                    continue;
                }
                if worker.recheck_pending(&shared).is_err() {
                    secret_scan::status(&shared, SecretScanStatus::Error);
                }
            }
            Job::Scan { folder, full } => {
                if !shared.secret_scan.view().settings.enabled {
                    shared.secret_scan.running.store(false, Ordering::Release);
                    continue;
                }
                worker.folder = Some(folder.clone());
                if let Err(error) = worker.scan(&shared, &folder, full)
                    && error.code != ErrorCode::KeyService
                {
                    secret_scan::status(&shared, SecretScanStatus::Error);
                }
                shared.secret_scan.running.store(false, Ordering::Release);
            }
            Job::Redact { finding_id, reply } => {
                let _ = reply.send(worker.redact(&shared, runtime, &finding_id));
            }
            Job::Stop => break,
        }
        if worker.cache.save(&cache_path).is_err() {
            secret_scan::status(&shared, SecretScanStatus::Error);
        }
    }
}

fn inputs(shared: &Shared) -> Result<Vec<VaultScanSecret>, CoreError> {
    let Some(keys) = &shared.config.keys else {
        return Err(failure("No Vault is configured."));
    };
    let secrets = keys.scan_secrets().map_err(|error| {
        if matches!(
            error,
            vw_keys::KeyError::Vault(vw_vault::VaultError::Locked | vw_vault::VaultError::NoVault)
        ) {
            secret_scan::status(shared, SecretScanStatus::Locked);
            CoreError::new(ErrorCode::KeyService, "Waits for the Vault to be unlocked.")
        } else {
            failure("Could not read the Vault for the session log check.")
        }
    })?;
    let (secrets, skipped) = filter_inputs(secrets);
    shared
        .secret_scan
        .view
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .skipped_secrets = skipped;
    Ok(secrets)
}

/// True when the session log file `path` lies in the configured session log `folder`.
///
/// Both sides are compared in their canonical forms (`std::fs::canonicalize`), because one folder can be written
/// several ways: the folder may be configured through a symlinked folder (or, on macOS, below the `/var` link),
/// and on Windows `canonicalize` answers `\\?\C:\...` with long names and the on-disk case, while the folder is
/// configured as `C:\...` (possibly with a short name or another case). The file is resolved too, so a file that
/// is a link to a file elsewhere, or a `..` hop out of the folder, stays outside. A side that cannot be
/// canonicalized (missing, no access) means outside: a redaction is refused rather than allowed on a guess.
pub(crate) fn inside_log_folder(folder: &Path, path: &Path) -> bool {
    let Ok(folder) = folder.canonicalize() else {
        return false;
    };
    path.canonicalize()
        .is_ok_and(|path| path.starts_with(&folder))
}

pub(crate) fn filter_inputs(mut secrets: Vec<VaultScanSecret>) -> (Vec<VaultScanSecret>, u32) {
    let total = secrets.len();
    secrets.retain(|secret| {
        let value = secret.value.expose();
        value.chars().count() >= 8 && zxcvbn::zxcvbn(value, &[]).guesses_log10() >= 16.0
    });
    let skipped = count(total - secrets.len());
    (secrets, skipped)
}

fn owned(secrets: &[VaultScanSecret]) -> Vec<vw_logscan::Secret> {
    secrets
        .iter()
        .map(|secret| vw_logscan::Secret::new(&secret.id, secret.value.expose().to_owned()))
        .collect()
}

fn values(secrets: &[VaultScanSecret]) -> Vec<&str> {
    secrets.iter().map(|secret| secret.value.expose()).collect()
}

fn finding_id(chat_id: &str) -> String {
    format!("secret-leak-{}", blake3::hash(chat_id.as_bytes()).to_hex())
}

fn failure(message: &str) -> CoreError {
    CoreError::new(ErrorCode::Internal, message)
}
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl Worker {
    fn scan(
        &mut self,
        shared: &std::sync::Arc<Shared>,
        folder: &Path,
        full: bool,
    ) -> Result<(), CoreError> {
        let secrets = inputs(shared)?;
        self.restrict_inputs(shared, &secrets);
        if secrets.is_empty() {
            secret_scan::status(shared, SecretScanStatus::NoSecrets);
            return Ok(());
        }
        let fingerprint = fingerprint(&secrets);
        let mut seen = BTreeSet::new();
        let mut failed = 0;
        let mut errors = Vec::new();
        for entry in walkdir::WalkDir::new(folder).follow_links(false) {
            if !yield_to_user(shared) {
                return Ok(());
            }
            let Ok(entry) =
                entry.map_err(|error| errors.push(error.path().unwrap_or(folder).to_path_buf()))
            else {
                failed += 1;
                continue;
            };
            if !entry.file_type().is_file()
                || !matches!(
                    entry.path().extension().and_then(|ext| ext.to_str()),
                    Some("json" | "jsonl")
                )
            {
                continue;
            }
            let path = entry.into_path();
            if crate::secret_scan_cache::derived_index(folder, &path) {
                continue;
            }
            seen.insert(path.clone());
            let Ok(stamp) = Stamp::read(&path) else {
                errors.push(path);
                failed += 1;
                continue;
            };
            if !full && !self.cache.changed(&path, &stamp, &fingerprint) {
                continue;
            }
            let report = vw_logscan::scan_files(vec![path.clone()], owned(&secrets))
                .map_err(|_| failure("Could not check the session log file."))?;
            if !report.skipped_files.is_empty() {
                errors.push(path.clone());
                failed += 1;
                self.cache.files.remove(&path);
                continue;
            }
            let hits = hits(&report, &secrets);
            // Only Vibe metadata establishes chat ownership. Unattributed sidecars cannot acquire redaction authority.
            if let Some((chat_id, title)) = chat(folder, &path) {
                let authority = finding_id(&chat_id);
                self.cache.files.insert(
                    path.clone(),
                    CachedFile {
                        stamp,
                        fingerprint: fingerprint.clone(),
                        chat_id,
                        title: withhold_parts(&title, &values(&secrets)),
                        hits,
                        places: count(report.place_count()),
                    },
                );
                self.reports
                    .entry(authority)
                    .or_default()
                    .insert(path, report);
            } else if hits.is_empty() {
                self.cache.files.insert(
                    path,
                    CachedFile {
                        stamp,
                        fingerprint: fingerprint.clone(),
                        chat_id: String::new(),
                        title: String::new(),
                        hits,
                        places: 0,
                    },
                );
            } else {
                errors.push(path);
                failed += 1;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.cache.files.retain(|path, _| seen.contains(path));
        for reports in self.reports.values_mut() {
            reports.retain(|path, _| seen.contains(path));
        }
        self.drop_stale_findings();
        self.refresh(shared);
        Self::checked(shared, failed, errors);
        Ok(())
    }

    /// A chat whose files are gone (deleted, the remedy for a sealed Vibe 3.0 store) or no longer hold a value
    /// loses its finding in this check, not one check later (the same rule as `restrict_inputs`).
    fn drop_stale_findings(&mut self) {
        let files = &self.cache.files;
        self.cache.findings.retain(|finding| {
            finding.state == SecretLeakState::Removed
                || files
                    .values()
                    .any(|file| file.chat_id == finding.chat_id && !file.hits.is_empty())
        });
    }

    fn checked(shared: &Shared, failed: u32, errors: Vec<PathBuf>) {
        *shared
            .secret_scan
            .errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = errors;
        let mut view = shared
            .secret_scan
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        view.checked_ms = Some(
            i64::try_from(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
            )
            .unwrap_or(i64::MAX),
        );
        view.failed_files = failed;
        view.status = if view.settings.enabled {
            SecretScanStatus::Checked
        } else {
            SecretScanStatus::Off
        };
        drop(view);
        secret_scan::publish(shared);
    }

    fn refresh(&mut self, shared: &Shared) {
        // A locked vault leaves nothing to compare: the cached titles were withheld when they were checked.
        let secrets = inputs(shared)
            .map(|s| filter_inputs(s).0)
            .unwrap_or_default();
        let mut chats: BTreeMap<String, ChatHits> = BTreeMap::new();
        for (path, file) in self
            .cache
            .files
            .iter()
            .filter(|(_, file)| !file.hits.is_empty())
        {
            let entry = chats
                .entry(file.chat_id.clone())
                .or_insert_with(|| ChatHits {
                    title: file.title.clone(),
                    secrets: BTreeMap::new(),
                    files: 0,
                    places: 0,
                    redactable: false,
                });
            entry.files += 1;
            entry.places += file.places;
            entry.redactable |= !self.sealed_file(path);
            for (id, hit) in &file.hits {
                let old = entry
                    .secrets
                    .entry(id.clone())
                    .or_insert_with(|| SecretLeakName {
                        places: 0,
                        ..hit.clone()
                    });
                old.places += hit.places;
            }
        }
        for (chat_id, chat) in &chats {
            let finding_id = finding_id(chat_id);
            let state = if shared.channels.has_channel(chat_id)
                || shared.leases.foreign(chat_id).ok().flatten().is_some()
            {
                SecretLeakState::ChatOpen
            } else {
                SecretLeakState::Found
            };
            let mut finding = SecretLeakFinding {
                finding_id: finding_id.clone(),
                chat_id: chat_id.clone(),
                // The live title only when there are values to withhold from it; else the cached, withheld one.
                chat_title: withhold_parts(
                    &clean(
                        &(!secrets.is_empty())
                            .then(|| shared.names.title_of(&shared.store, chat_id))
                            .flatten()
                            .unwrap_or_else(|| chat.title.clone()),
                    ),
                    &values(&secrets),
                ),
                secrets: chat.secrets.values().cloned().collect(),
                places: chat.places,
                state,
                redacted_files: 0,
                files: chat.files,
                changed_files: 0,
                format_files: 0,
                unreadable_files: 0,
                redactable: chat.redactable,
            };
            if let Some(old) = self
                .cache
                .findings
                .iter_mut()
                .find(|old| old.finding_id == finding_id)
            {
                for name in old
                    .secrets
                    .iter()
                    .filter(|name| name.state == SecretLeakState::Removed)
                {
                    if !finding.secrets.iter().any(|current| {
                        current.name == name.name
                            && current.website_password == name.website_password
                    }) {
                        finding.secrets.push(name.clone());
                    }
                }
                *old = finding;
            } else {
                self.cache.findings.push(finding);
            }
        }
        let mut view = shared
            .secret_scan
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        view.chats = count(chats.len());
        view.places = chats.values().map(|chat| chat.places).sum();
        view.findings.clone_from(&self.cache.findings);
        drop(view);
        secret_scan::publish(shared);
    }
}

fn yield_to_user(shared: &Shared) -> bool {
    loop {
        if shared.secret_scan.stopped.load(Ordering::Acquire)
            || !shared.secret_scan.view().settings.enabled
        {
            return false;
        }
        if !secret_scan::busy(shared) && shared.secret_scan.idle() >= Duration::from_millis(250) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn hits(report: &ScanReport, secrets: &[VaultScanSecret]) -> BTreeMap<String, SecretLeakName> {
    report
        .secrets
        .iter()
        .filter(|secret| !secret.locations.is_empty())
        .filter_map(|secret| {
            let input = secrets.iter().find(|input| input.id == secret.id)?;
            Some((
                secret.id.clone(),
                SecretLeakName {
                    name: clean(&input.name),
                    website_password: input.website_password,
                    login_id: input.login_id.clone(),
                    places: count(secret.locations.len()),
                    state: SecretLeakState::Found,
                },
            ))
        })
        .collect()
}
