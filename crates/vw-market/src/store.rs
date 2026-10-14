//! The installed state: where packs and plugins live in Vibe Work's own Vibe home, their receipts, listing and
//! removal.
//!
//! Layout under `home` (Vibe Work's `VIBE_HOME`, never `~/.vibe`):
//! - `plugins/<id>/` – the plugin Vibe loads (user scope, `harness_files/_paths.py:7`), with our receipt
//!   [`RECEIPT`];
//! - `vw-market/<id>/` – the entry's data (e.g. a browser download), outside the plugin so Vibe's package store
//!   (10,000 entries / 256 MiB per plugin) never copies it;
//! - `vw-market/.staging/<id>.<unique>/` – a plugin being built, one fresh folder per install (outside `plugins/`, so
//!   Vibe never loads half a plugin);
//! - `vw-market/.trash/` – a removed plugin on its way out (a rename first, so it disappears from `plugins/` at once).
//!
//! Depends on: `serde_json`, `std::fs`. Called by: `install`, vw-core-api.
//! Invariants:
//! - A folder without our receipt is never replaced or removed ([`MarketError::Foreign`]).
//! - Every id is checked by [`checked_id`] before it is joined to a folder (an absolute or `..` id from a
//!   request must not reach `Path::join`).
//! - Receipts list the SHA-256 of every file the marketplace wrote; [`Store::changed`] compares a pack against it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::MarketError;

/// The receipt file inside an installed plugin folder.
pub const RECEIPT: &str = ".vw-market.json";

/// What the marketplace wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    /// The catalog entry.
    pub entry_id: String,
    /// The entry's version when installed.
    pub version: String,
    /// The catalog's version when installed.
    pub catalog_version: String,
    /// Milliseconds since the Unix epoch.
    pub installed_at_ms: u64,
    /// Every file written, relative path -> SHA-256 (the receipt itself excluded).
    pub files: BTreeMap<String, String>,
}

/// The marketplace's folders inside one Vibe home.
#[derive(Debug, Clone)]
pub struct Store {
    home: PathBuf,
}

impl Store {
    /// The store in `home` (Vibe Work's own Vibe home).
    #[must_use]
    pub fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
        }
    }

    /// `plugins/`.
    #[must_use]
    pub fn plugins_dir(&self) -> PathBuf {
        self.home.join("plugins")
    }

    /// `plugins/<id>/`.
    ///
    /// # Errors
    /// [`MarketError::InvalidId`] unless `id` is a plain plugin name (see [`checked_id`]).
    pub fn plugin_dir(&self, id: &str) -> Result<PathBuf, MarketError> {
        Ok(self.plugins_dir().join(checked_id(id)?))
    }

    /// `vw-market/<id>/` (the entry's data).
    ///
    /// # Errors
    /// [`MarketError::InvalidId`] as [`Store::plugin_dir`].
    pub fn data_dir(&self, id: &str) -> Result<PathBuf, MarketError> {
        Ok(self.home.join("vw-market").join(checked_id(id)?))
    }

    fn staging_root(&self) -> PathBuf {
        self.home.join("vw-market").join(".staging")
    }

    fn trash_dir(&self) -> PathBuf {
        self.home.join("vw-market").join(".trash")
    }

    /// Creates a fresh, empty staging folder for one install of `id` (`vw-market/.staging/<id>.<unique>/`). The leaf
    /// is made with `create_dir`, so it is new: two installs never share one, and nothing planted beforehand is used.
    ///
    /// # Errors
    /// [`MarketError::InvalidId`]; I/O errors.
    pub fn begin_staging(&self, id: &str) -> Result<PathBuf, MarketError> {
        let id = checked_id(id)?;
        let root = self.staging_root();
        std::fs::create_dir_all(&root)
            .map_err(|e| MarketError::io("create the staging folder", e))?;
        let path = root.join(format!("{id}.{}", unique()));
        std::fs::create_dir(&path).map_err(|e| MarketError::io("create the staging folder", e))?;
        Ok(path)
    }

    /// Deletes a staging folder made by [`Store::begin_staging`] (a cancelled or failed install). Anything that is not
    /// directly inside the staging root is left alone.
    pub fn clear_staging(&self, path: &Path) {
        if path.parent() == Some(self.staging_root().as_path()) {
            let _ = remove_tree(path);
        }
    }

    /// The receipt of `id`: `Ok(None)` when not installed.
    ///
    /// # Errors
    /// [`MarketError::InvalidId`]; [`MarketError::Foreign`] when `plugins/<id>` exists without a readable receipt for
    /// `id`.
    pub fn receipt(&self, id: &str) -> Result<Option<Receipt>, MarketError> {
        let dir = self.plugin_dir(id)?;
        if !dir.exists() && !dir.is_symlink() {
            return Ok(None);
        }
        if dir.is_symlink() || !dir.is_dir() {
            return Err(MarketError::Foreign(id.to_owned()));
        }
        let text = std::fs::read_to_string(dir.join(RECEIPT))
            .map_err(|_| MarketError::Foreign(id.to_owned()))?;
        let receipt: Receipt =
            serde_json::from_str(&text).map_err(|_| MarketError::Foreign(id.to_owned()))?;
        if receipt.entry_id != id {
            return Err(MarketError::Foreign(id.to_owned()));
        }
        Ok(Some(receipt))
    }

    /// The MCP server names declared by an installed entry, from its installed `mcp.json`.
    /// Skill-only packs have none; the current catalog may differ from the installed version.
    ///
    /// # Errors
    /// Receipt errors, [`MarketError::NotInstalled`], or an unreadable/invalid installed `mcp.json`.
    pub fn mcp_server_names(&self, id: &str) -> Result<Vec<String>, MarketError> {
        let receipt = self
            .receipt(id)?
            .ok_or_else(|| MarketError::NotInstalled(id.to_owned()))?;
        if !receipt.files.contains_key("mcp.json") {
            return Ok(Vec::new());
        }
        let path = self.plugin_dir(id)?.join("mcp.json");
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|e| MarketError::io("check installed mcp.json", e))?;
        if !metadata.is_file() || metadata.len() > 1_048_576 {
            return Err(MarketError::io(
                "check installed mcp.json",
                std::io::Error::new(std::io::ErrorKind::InvalidData, "not a small regular file"),
            ));
        }
        let bytes =
            std::fs::read(path).map_err(|e| MarketError::io("read installed mcp.json", e))?;
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
            MarketError::io(
                "read installed mcp.json",
                std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid JSON"),
            )
        })?;
        let servers = manifest["mcpServers"].as_object().ok_or_else(|| {
            MarketError::io(
                "read installed mcp.json",
                std::io::Error::new(std::io::ErrorKind::InvalidData, "missing mcpServers object"),
            )
        })?;
        Ok(servers.keys().cloned().collect())
    }

    /// Every receipt in `plugins/` (folders without one, or with a name that is not a plain plugin name, are skipped:
    /// they are not ours).
    #[must_use]
    pub fn installed(&self) -> BTreeMap<String, Receipt> {
        let mut out = BTreeMap::new();
        let Ok(dir) = std::fs::read_dir(self.plugins_dir()) else {
            return out;
        };
        for item in dir.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            if let Ok(Some(receipt)) = self.receipt(&name) {
                out.insert(name, receipt);
            }
        }
        out
    }

    /// True when a file listed in the receipt is missing or differs (for small packs; callers skip big plugins). A
    /// receipt path that is not a plain relative path counts as changed and is never read.
    #[must_use]
    pub fn changed(&self, receipt: &Receipt) -> bool {
        let Ok(dir) = self.plugin_dir(&receipt.entry_id) else {
            return true;
        };
        receipt.files.iter().any(|(path, sha)| {
            crate::catalog_rules::check_path(path).is_err()
                || std::fs::read(dir.join(path))
                    .map_or(true, |bytes| crate::digest::sha256_hex(&bytes) != *sha)
        })
    }

    /// Moves the finished `staging` folder of `id` into `plugins/`, replacing an older install of ours.
    ///
    /// # Errors
    /// [`MarketError::InvalidId`]; [`MarketError::Foreign`] when a folder of that name is not ours; I/O errors.
    pub fn commit(&self, id: &str, staging: &Path) -> Result<(), MarketError> {
        self.receipt(id)?;
        std::fs::create_dir_all(self.plugins_dir())
            .map_err(|e| MarketError::io("create plugins/", e))?;
        let target = self.plugin_dir(id)?;
        if target.exists() {
            self.discard(id)?;
        }
        std::fs::rename(staging, &target).map_err(|e| MarketError::io("move into plugins/", e))
    }

    /// Removes the installed plugin `id` (and its data folder when `with_data`).
    ///
    /// # Errors
    /// [`MarketError::InvalidId`], [`MarketError::NotInstalled`], [`MarketError::Foreign`], I/O errors.
    pub fn remove(&self, id: &str, with_data: bool) -> Result<(), MarketError> {
        if self.receipt(id)?.is_none() {
            return Err(MarketError::NotInstalled(id.to_owned()));
        }
        self.discard(id)?;
        if with_data {
            self.remove_data(id)?;
        }
        Ok(())
    }

    /// Deletes the data folder of `id`, if present. Call after reloading sessions to stop its MCP servers.
    ///
    /// # Errors
    /// [`MarketError::InvalidId`]; I/O errors. The plugin's installed state is unaffected.
    pub fn remove_data(&self, id: &str) -> Result<(), MarketError> {
        remove_tree(&self.data_dir(id)?).map_err(|e| MarketError::io("remove the data folder", e))
    }

    /// Renames `plugins/<id>` into the trash (gone from Vibe's view at once), then deletes it.
    fn discard(&self, id: &str) -> Result<(), MarketError> {
        let trash = self.trash_dir();
        std::fs::create_dir_all(&trash)
            .map_err(|e| MarketError::io("create the trash folder", e))?;
        let moved = trash.join(format!("{}.{}", checked_id(id)?, unique()));
        std::fs::rename(self.plugin_dir(id)?, &moved)
            .map_err(|e| MarketError::io("move the old plugin out", e))?;
        remove_tree(&moved).map_err(|e| MarketError::io("delete the old plugin", e))
    }
}

/// `id` when it is a plain plugin name (`catalog::is_plugin_name`: a-z, 0-9, single dashes, 1-48 characters), the only
/// form ever joined to a folder. This is what keeps a request's id (UI, Debug API) or a name read from disk from
/// becoming `..`, an absolute path, a drive or UNC prefix (`Path::join` replaces the base with an absolute path).
///
/// # Errors
/// [`MarketError::InvalidId`].
pub fn checked_id(id: &str) -> Result<&str, MarketError> {
    if crate::catalog::is_plugin_name(id) {
        Ok(id)
    } else {
        Err(MarketError::InvalidId(id.chars().take(64).collect()))
    }
}

/// A process-unique suffix: time in nanoseconds plus a counter.
fn unique() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{nanos}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Deletes a folder tree, making read-only entries writable first (Unix). Missing = done. Links are removed, never
/// followed.
pub(crate) fn remove_tree(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(meta) if meta.is_dir() => {
            make_writable(path);
            for item in std::fs::read_dir(path)? {
                remove_tree(&item?.path())?;
            }
            std::fs::remove_dir(path)
        }
        Ok(_) => std::fs::remove_file(path),
    }
}

#[cfg(unix)]
fn make_writable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o700);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn make_writable(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        // Windows: clears the read-only attribute only.
        perms.set_readonly(false);
        let _ = std::fs::set_permissions(path, perms);
    }
}
