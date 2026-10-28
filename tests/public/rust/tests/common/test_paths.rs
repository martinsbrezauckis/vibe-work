//! One disposable filesystem root for real-server fixtures, shared across both test workspaces.
//! Uses only std; scratch helpers and the sandbox launcher/tap use it to agree on visible folders.

use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

/// An absolute override or checkout-specific root, shared with the sandbox and re-executed tests.
pub fn root() -> PathBuf {
    let base = match std::env::var_os("VW_TEST_ROOT") {
        Some(value) => {
            let path = PathBuf::from(value);
            assert!(
                path.is_absolute(),
                "VW_TEST_ROOT must be an absolute folder"
            );
            path
        }
        None => {
            let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .canonicalize()
                .unwrap();
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            repo.hash(&mut hash);
            // Re-executed tests can replace TMPDIR and HOME; keep their parent's fixture root.
            repo.parent()
                .unwrap()
                .join(format!(".vw-tests-{:016x}", hash.finish()))
        }
    };
    std::fs::create_dir_all(&base).unwrap();
    let path = base.canonicalize().unwrap();
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// Resolve existing ancestors too: a symlink or `..` must not disguise a folder outside the bind.
pub fn require_inside(root: &Path, folder: &Path) -> Result<(), String> {
    let mut resolved = PathBuf::new();
    if folder.is_absolute() {
        for part in folder.components() {
            match part {
                Component::ParentDir => {
                    resolved.pop();
                }
                Component::CurDir => {}
                part => resolved.push(part.as_os_str()),
            }
            if resolved.symlink_metadata().is_ok() {
                resolved = resolved.canonicalize().map_err(|e| {
                    format!(
                        "real Vibe test folder {} cannot be resolved: {e}",
                        folder.display()
                    )
                })?;
            }
        }
        if resolved.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
            return Ok(());
        }
    }
    Err(format!(
        "real Vibe test folder {} is outside the sandbox test root {}; create it with the shared scratch helper",
        folder.display(),
        root.display()
    ))
}
