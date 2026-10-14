//! Installing one catalog entry: download, verify, unpack, assemble in a staging folder, run the declared setup
//! step, write the receipt, move into `plugins/`.
//!
//! Depends on: `catalog`, `fetch`, `archive`, `assemble`, `store`, `digest`, tokio (the setup step). Called by:
//! vw-core-api (`marketplace install|update`, as an owned task).
//! Invariants:
//! - Every downloaded byte is checked against its pin before anything is written; one mismatch stops the install
//!   with nothing in `plugins/` ([`MarketError::HashMismatch`]).
//! - Work happens in the staging folder; a failure, an error or a cancelled task (the future dropped) removes it
//!   ([`StagingGuard`]). Each install gets its own new staging folder ([`Store::begin_staging`]); `plugins/<id>`
//!   changes only in [`Store::commit`]'s rename.
//! - Every file is created with `create_new`: an entry that already exists (a second write to one path, a case-only
//!   twin on a case-insensitive disk, anything planted) fails the install instead of being overwritten or followed.
//! - No vendor code runs except a `plugin` entry's declared setup step, started with the pinned runtime, a cleaned
//!   environment (no keys, only what a downloader needs) and a timeout; its output is not shown beyond a short
//!   tail on failure.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use crate::archive;
use crate::assemble::{self, Written};
use crate::catalog::{Catalog, Entry, Kind, Pack, PluginSpec, PostInstall};
use crate::digest;
use crate::error::MarketError;
use crate::fetch::Fetcher;
use crate::store::{Receipt, Store};

/// One progress report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// `download`, `unpack`, `setup` or `finish`.
    pub phase: &'static str,
    /// What (a catalog-relative name or the setup step's words).
    pub what: String,
    /// Bytes so far (downloads).
    pub done: Option<u64>,
    /// Bytes expected (downloads).
    pub total: Option<u64>,
}

/// Receives progress.
pub type OnStep<'a> = &'a (dyn Fn(Step) + Send + Sync);

/// Installs (or updates) entries of one catalog into one store.
pub struct Installer<'a> {
    /// The catalog.
    pub catalog: &'a Catalog,
    /// Where.
    pub store: &'a Store,
    /// How bytes are downloaded.
    pub fetcher: &'a dyn Fetcher,
    /// The platform key (`catalog::current_target()` in the app).
    pub target: Option<&'a str>,
}

/// Removes the staging folder when dropped (an error, or the task aborted mid-install), unless disarmed.
struct StagingGuard<'a> {
    store: &'a Store,
    path: std::path::PathBuf,
    armed: bool,
}

impl Drop for StagingGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.store.clear_staging(&self.path);
        }
    }
}

impl Installer<'_> {
    /// Installs `id`, replacing an older install of it.
    ///
    /// # Errors
    /// Any [`MarketError`]; on error nothing in `plugins/` changed.
    pub async fn install(&self, id: &str, on: OnStep<'_>) -> Result<Receipt, MarketError> {
        let entry = self
            .catalog
            .entry(id)
            .ok_or_else(|| MarketError::UnknownEntry(id.to_owned()))?;
        self.store.receipt(id)?;
        // A fresh folder per install (no shared staging between two installs, nothing planted reused).
        let root = self.store.begin_staging(id)?;
        let mut guard = StagingGuard {
            store: self.store,
            path: root.clone(),
            armed: true,
        };
        let mut written = Written::default();
        let version = match entry.kind {
            Kind::Pack => {
                let pack = entry
                    .pack
                    .as_ref()
                    .ok_or_else(|| MarketError::Catalog("pack block missing".into()))?;
                self.build_pack(entry, pack, &root, &mut written, on)
                    .await?;
                pack.version.clone()
            }
            Kind::Plugin => {
                let plugin = entry
                    .plugin
                    .as_ref()
                    .ok_or_else(|| MarketError::Catalog("plugin block missing".into()))?;
                self.build_plugin(entry, plugin, &root, &mut written, on)
                    .await?;
                plugin.version.clone()
            }
            Kind::Server => {
                return Err(MarketError::NotInstallable(
                    "a remote server is added from the Connectors page, not installed".to_owned(),
                ));
            }
        };
        written.write(
            &root,
            "plugin.json",
            &assemble::manifest(entry, &version),
            false,
        )?;
        on(Step {
            phase: "finish",
            what: entry.name.clone(),
            done: None,
            total: None,
        });
        let receipt = Receipt {
            entry_id: id.to_owned(),
            version,
            catalog_version: self.catalog.version.clone(),
            installed_at_ms: now_ms(),
            files: written.files,
        };
        let text =
            serde_json::to_vec_pretty(&receipt).map_err(|e| MarketError::Catalog(e.to_string()))?;
        crate::assemble::write_new(&root.join(crate::store::RECEIPT), &text)
            .map_err(|e| MarketError::io("write the receipt", e))?;
        self.store.commit(id, &root)?;
        guard.armed = false;
        Ok(receipt)
    }

    async fn build_pack(
        &self,
        entry: &Entry,
        pack: &Pack,
        root: &Path,
        written: &mut Written,
        on: OnStep<'_>,
    ) -> Result<(), MarketError> {
        if let Some(source) = &pack.source {
            // The catalog checks `commit` (40 hex digits) only for a pack that has vendor files, so a pack with
            // just servers can carry any text here: take 12 CHARACTERS, never a byte slice, which panics on a
            // short value or a cut inside a multi-byte character.
            let short_commit: String = source.commit.chars().take(12).collect();
            let what = format!("{} at {short_commit}", source.repo);
            let bytes = self
                .download(&source.url(), source.max_bytes, &what, on)
                .await?;
            on(Step {
                phase: "unpack",
                what: what.clone(),
                done: None,
                total: None,
            });
            let mut wanted: Vec<&str> = pack.files.iter().map(|f| f.from.as_str()).collect();
            if let Some(licence) = &pack.licence_file {
                wanted.push(licence.from.as_str());
            }
            let found =
                archive::read_listed(&bytes, crate::catalog::ArchiveFormat::TarGz, &wanted, true)?;
            let mut out: BTreeMap<String, Vec<u8>> = BTreeMap::new();
            for file in &pack.files {
                let data = found.get(&file.from).cloned().unwrap_or_default();
                if digest::sha256_hex(&data) != file.sha256 {
                    return Err(MarketError::HashMismatch {
                        what: file.to.clone(),
                    });
                }
                out.insert(file.to.clone(), data);
            }
            for edit in &pack.edits {
                apply_edit(&mut out, edit)?;
            }
            for edit in &pack.edits {
                let data = out.get(&edit.file).map(Vec::as_slice).unwrap_or_default();
                if digest::sha256_hex(data) != edit.result_sha256 {
                    return Err(MarketError::HashMismatch {
                        what: format!("{} (edited)", edit.file),
                    });
                }
            }
            if let Some(licence) = &pack.licence_file {
                let data = found
                    .get(&licence.from)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if digest::sha256_hex(data) != licence.sha256 {
                    return Err(MarketError::HashMismatch {
                        what: "LICENSE".to_owned(),
                    });
                }
                written.write(root, "LICENSE", data, false)?;
            }
            for (to, data) in &out {
                written.write(root, to, data, false)?;
            }
        }
        if !pack.mcp_servers.is_empty() {
            let data = self.store.data_dir(&entry.id)?;
            written.write(
                root,
                "mcp.json",
                &assemble::mcp_json(&pack.mcp_servers, None, &data),
                false,
            )?;
        }
        Ok(())
    }

    async fn build_plugin(
        &self,
        entry: &Entry,
        plugin: &PluginSpec,
        root: &Path,
        written: &mut Written,
        on: OnStep<'_>,
    ) -> Result<(), MarketError> {
        let target_key = self.target.ok_or_else(|| {
            MarketError::NotInstallable(
                "this computer's platform has no runtime in the catalog".to_owned(),
            )
        })?;
        let target = plugin.runtime.targets.get(target_key).ok_or_else(|| {
            MarketError::NotInstallable(format!(
                "no {} runtime for {target_key} in the catalog",
                plugin.runtime.name
            ))
        })?;
        let what = format!("{} {}", plugin.runtime.name, plugin.runtime.version);
        let bytes = self.download(&target.url, target.bytes, &what, on).await?;
        if digest::sha256_hex(&bytes) != target.sha256 {
            return Err(MarketError::HashMismatch { what });
        }
        on(Step {
            phase: "unpack",
            what: what.clone(),
            done: None,
            total: None,
        });
        let wanted: Vec<&str> = target.files.iter().map(|m| m.from.as_str()).collect();
        let found = archive::read_listed(&bytes, target.format, &wanted, false)?;
        drop(bytes);
        for member in &target.files {
            let data = found
                .get(&member.from)
                .map(Vec::as_slice)
                .unwrap_or_default();
            written.write(root, &member.to, data, member.executable)?;
        }
        drop(found);
        for package in &plugin.npm {
            let what = format!("{}@{}", package.name, package.version);
            let bytes = self
                .download(&package.url, package.max_bytes, &what, on)
                .await?;
            if !digest::matches_sri_sha512(&bytes, &package.integrity) {
                return Err(MarketError::HashMismatch { what });
            }
            on(Step {
                phase: "unpack",
                what: what.clone(),
                done: None,
                total: None,
            });
            for (path, data) in archive::read_npm(&bytes)? {
                written.write(
                    root,
                    &format!("node_modules/{}/{path}", package.name),
                    &data,
                    false,
                )?;
            }
        }
        for (to, name) in &plugin.bundled {
            let text = assemble::bundled(name).ok_or_else(|| {
                MarketError::Catalog(format!("bundled file {name} missing from this build"))
            })?;
            written.write(root, to, text.as_bytes(), false)?;
        }
        let data = self.store.data_dir(&entry.id)?;
        std::fs::create_dir_all(&data).map_err(|e| MarketError::io("create the data folder", e))?;
        written.write(
            root,
            "mcp.json",
            &assemble::mcp_json(&plugin.mcp_servers, Some(&target.executable), &data),
            false,
        )?;
        if let Some(step) = &plugin.post_install {
            on(Step {
                phase: "setup",
                what: step.what.clone(),
                done: None,
                total: None,
            });
            run_step(&root.join(&target.executable), step, root, &data).await?;
        }
        Ok(())
    }

    async fn download(
        &self,
        url: &str,
        max: u64,
        what: &str,
        on: OnStep<'_>,
    ) -> Result<Vec<u8>, MarketError> {
        let report = |done: u64| {
            on(Step {
                phase: "download",
                what: what.to_owned(),
                done: Some(done),
                total: Some(max),
            });
        };
        on(Step {
            phase: "download",
            what: what.to_owned(),
            done: Some(0),
            total: Some(max),
        });
        self.fetcher
            .get(url, max, &report)
            .await
            .map_err(|e| match e {
                MarketError::Download(reason) => MarketError::Download(format!("{what}: {reason}")),
                other => other,
            })
    }
}

fn apply_edit(
    files: &mut BTreeMap<String, Vec<u8>>,
    edit: &crate::catalog::Edit,
) -> Result<(), MarketError> {
    let data = files
        .get_mut(&edit.file)
        .ok_or_else(|| MarketError::Catalog(format!("{}: no such file", edit.file)))?;
    let text = String::from_utf8(std::mem::take(data))
        .map_err(|_| MarketError::Catalog(format!("{}: not text", edit.file)))?;
    if text.matches(edit.find.as_str()).count() != 1 {
        return Err(MarketError::HashMismatch {
            what: format!("{} (edit anchor)", edit.file),
        });
    }
    *data = text
        .replacen(edit.find.as_str(), &edit.replace, 1)
        .into_bytes();
    Ok(())
}

/// Environment variables passed to the setup step (a downloader's needs: no keys, no tokens).
const STEP_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "TEMP",
    "TMP",
    "TMPDIR",
    "SYSTEMROOT",
    "SystemRoot",
    "WINDIR",
    "ComSpec",
    "LANG",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "XDG_CACHE_HOME",
];

async fn run_step(
    runtime: &Path,
    step: &PostInstall,
    root: &Path,
    data: &Path,
) -> Result<(), MarketError> {
    let mut command = tokio::process::Command::new(runtime);
    command.env_clear();
    for name in STEP_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    for (name, value) in &step.env {
        command.env(name, assemble::expand_step(value, root, data));
    }
    command
        .args(
            step.args
                .iter()
                .map(|a| assemble::expand_step(a, root, data)),
        )
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = command
        .spawn()
        .map_err(|e| MarketError::PostInstall(format!("could not start: {e}")))?;
    let output = tokio::time::timeout(
        Duration::from_secs(step.timeout_secs),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| {
        MarketError::PostInstall(format!(
            "{} took longer than {} s",
            step.what, step.timeout_secs
        ))
    })?
    .map_err(|e| MarketError::PostInstall(e.to_string()))?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    });
    let tail: String = text
        .chars()
        .rev()
        .take(400)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Err(MarketError::PostInstall(format!(
        "{} failed: {}",
        step.what,
        tail.trim()
    )))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
