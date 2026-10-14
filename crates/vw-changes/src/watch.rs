//! Live refresh: a file watcher on the repository root that calls back once per burst of relevant changes.
//!
//! Depends on: `notify` 8.2.0 (CC0-1.0; inotify / `ReadDirectoryChangesW` / `FSEvents`), std. Called by:
//! vw-core-api (one watcher per tracked session).
//! Invariants:
//! - Reads (`Access` events) never count: our own refresh reads every changed file, and counting reads
//!   would make each refresh trigger the next.
//! - Events under the git directory and under ignored paths (git's own list, reloaded when a `.gitignore`
//!   changes) are dropped, so a build writing `target/` or `node_modules/` costs nothing.
//! - The callback runs on the watcher's own thread, at most once per `quiet` pause and at least once per
//!   `max_wait` while events keep coming. Dropping the [`Watcher`] stops the thread.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher as _};

use crate::error::ChangesError;

/// Timing of the callback.
#[derive(Debug, Clone, Copy)]
pub struct Debounce {
    /// Calls back this long after the last relevant event.
    pub quiet: Duration,
    /// Calls back at the latest this long after the first event of a burst.
    pub max_wait: Duration,
}

impl Default for Debounce {
    fn default() -> Self {
        Self {
            quiet: Duration::from_millis(250),
            max_wait: Duration::from_secs(1),
        }
    }
}

/// A running watcher. Drop it to stop.
pub struct Watcher {
    _inner: notify::RecommendedWatcher,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Watcher")
    }
}

/// Paths to drop: the git directory and git's ignored list (relative, `dir/` for folders).
struct Filter {
    root: PathBuf,
    git_dir: PathBuf,
    ignored: Vec<String>,
}

impl Filter {
    fn relevant(&self, path: &Path) -> bool {
        if path.starts_with(&self.git_dir) {
            return false;
        }
        let Ok(rel) = path.strip_prefix(&self.root) else {
            return false;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if rel == ".git" || rel.starts_with(".git/") {
            return false;
        }
        !self.ignored.iter().any(|ignored| {
            if let Some(dir) = ignored.strip_suffix('/') {
                rel == dir || rel.starts_with(ignored.as_str())
            } else {
                rel == *ignored
            }
        })
    }
}

/// Starts watching `root`. `ignored` lists git's ignored paths (called at start and after a `.gitignore`
/// changes); `on_change` runs once per burst of relevant events.
///
/// # Errors
/// `Io` when the OS watcher cannot be created or cannot watch `root` (e.g. the inotify watch limit).
pub fn watch(
    root: PathBuf,
    git_dir: PathBuf,
    debounce: Debounce,
    ignored: impl Fn() -> Vec<String> + Send + 'static,
    on_change: impl Fn() + Send + 'static,
) -> Result<Watcher, ChangesError> {
    let (tx, rx) = channel::<notify::Result<notify::Event>>();
    let mut inner = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .map_err(|e| ChangesError::io("watch", &root, std::io::Error::other(e.to_string())))?;
    inner
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| ChangesError::io("watch", &root, std::io::Error::other(e.to_string())))?;
    std::thread::Builder::new()
        .name("vw-changes-watch".into())
        .spawn(move || {
            let mut filter = Filter {
                root,
                git_dir,
                ignored: ignored(),
            };
            let mut first: Option<Instant> = None;
            let mut last = Instant::now();
            loop {
                let wait = first.map_or(Duration::from_secs(3600), |start| {
                    let due = (last + debounce.quiet).min(start + debounce.max_wait);
                    due.saturating_duration_since(Instant::now())
                });
                match rx.recv_timeout(wait) {
                    Ok(Ok(event)) => {
                        if matches!(event.kind, EventKind::Access(_)) {
                            continue;
                        }
                        if event.paths.iter().any(|p| p.ends_with(".gitignore")) {
                            filter.ignored = ignored();
                        }
                        let counts = event.paths.is_empty()
                            || event.paths.iter().any(|p| filter.relevant(p));
                        if counts {
                            last = Instant::now();
                            first.get_or_insert(last);
                        }
                    }
                    // An overflow or a backend error: something changed that we cannot name.
                    Ok(Err(_)) => {
                        last = Instant::now();
                        first.get_or_insert(last);
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if first.take().is_some() {
                            on_change();
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .map_err(|e| ChangesError::io("start the watcher thread for", PathBuf::new(), e))?;
    Ok(Watcher { _inner: inner })
}
