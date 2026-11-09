//! Negative repository discovery cache. Uses fresh ancestor marker stats and a prior successful Git answer;
//! called by `repo::discover`. Repositories and uncertain filesystem reads always go through Git.
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key {
    folder: PathBuf,
    search_path: Option<OsString>,
}

fn cache() -> &'static Mutex<BTreeSet<Key>> {
    static CACHE: OnceLock<Mutex<BTreeSet<Key>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

pub(super) fn key(folder: &Path) -> Option<Key> {
    // Follow the folder itself first: a symlink can lead to a repository outside its lexical ancestors.
    let folder = folder.canonicalize().ok()?;
    for parent in folder.ancestors() {
        match parent.join(".git").symlink_metadata() {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            // A file, directory, link, or an unreadable marker may be a repository. Let Git decide every time.
            _ => return None,
        }
    }
    Some(Key {
        folder,
        search_path: std::env::var_os("PATH"),
    })
}

pub(super) fn known(key: &Key) -> bool {
    cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(key)
}

pub(super) fn remember(key: Option<Key>) {
    if let Some(key) = key {
        let mut cache = cache().lock().unwrap_or_else(PoisonError::into_inner);
        // Session churn must not accumulate every folder ever opened. Eviction merely costs a new Git query.
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(key);
    }
}
