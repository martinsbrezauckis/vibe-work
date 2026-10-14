//! Stable, domain-separated storage keys for session previews and the shared public browser, plus mount labels.
//!
//! Depends on: sha2. Called by: `service`, the Tauri engine (vw-webview), tests.
//! Invariants:
//! - A session key is the first 16 bytes of SHA-256(`"vibe-work-preview\0" + session id`): stable across runs,
//!   never the bare session id (which is a path segment chosen by Vibe). The browser key uses a separate domain.
//! - Every preview webview label starts with [`LABEL_PREFIX`]; the navigation guard of the app refuses any other
//!   non-`main` label.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Prefix of every preview webview label (`vw-preview-<key hex>-<generation>`).
pub const LABEL_PREFIX: &str = "vw-preview-";

/// Domain separator of the key hash.
const KEY_DOMAIN: &[u8] = b"vibe-work-preview\0";
/// A separate domain ensures no session id can alias the browser's shared store.
const SHARED_WEB_DOMAIN: &[u8] = b"vibe-work-browser-shared\0";

/// The per-session storage key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StorageKey([u8; 16]);

impl StorageKey {
    /// The durable public-browser storage key, distinct from every session key.
    #[must_use]
    pub fn shared_web() -> Self {
        let digest = Sha256::digest(SHARED_WEB_DOMAIN);
        let mut key = [0u8; 16];
        key.copy_from_slice(&digest[..16]);
        Self(key)
    }
    /// The key of `session_id`.
    #[must_use]
    pub fn of(session_id: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(KEY_DOMAIN);
        hasher.update(session_id.as_bytes());
        let digest = hasher.finalize();
        let mut key = [0u8; 16];
        key.copy_from_slice(&digest[..16]);
        Self(key)
    }

    /// The 16 bytes (the macOS `WKWebsiteDataStore` identifier).
    #[must_use]
    pub fn bytes(self) -> [u8; 16] {
        self.0
    }

    /// Lowercase hex (32 characters): the folder name and the label middle.
    #[must_use]
    pub fn hex(self) -> String {
        let mut text = String::with_capacity(32);
        for byte in self.0 {
            let _ = write!(text, "{byte:02x}");
        }
        text
    }

    /// The session's data folder under `root` (`root/<hex>`).
    #[must_use]
    pub fn data_dir(self, root: &Path) -> PathBuf {
        root.join(self.hex())
    }

    /// The webview label for mount number `generation` (a new label per mount, so a re-mount never waits for
    /// the previous webview's label to be released).
    #[must_use]
    pub fn label(self, generation: u64) -> String {
        format!("{LABEL_PREFIX}{}-{generation}", self.hex())
    }
}

/// True when `label` names a preview webview.
#[must_use]
pub fn is_preview_label(label: &str) -> bool {
    label.starts_with(LABEL_PREFIX)
}
