//! Bounded project tree watches. Depends on notify and core events; called by `git_browser`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::core::Shared;
use crate::event::CoreEvent;

struct ActiveWatch {
    _watcher: RecommendedWatcher,
    task: tokio::task::JoinHandle<()>,
}

/// Each project has one nonrecursive watcher and one bounded debounce channel.
#[derive(Default)]
pub(crate) struct BrowserWatches(Mutex<BTreeMap<String, ActiveWatch>>);

/// Read notifications and transient lock files are not edits to show. Index writes are.
fn changes_tree(event: &notify::Event) -> bool {
    if !matches!(
        event.kind,
        notify::EventKind::Create(_) | notify::EventKind::Modify(_) | notify::EventKind::Remove(_)
    ) {
        return false;
    }
    event.paths.iter().any(|path| {
        let lock_file = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("lock"));
        if lock_file {
            return false;
        }
        if path.file_name().is_some_and(|name| name == "index") {
            return matches!(
                event.kind,
                notify::EventKind::Create(_)
                    | notify::EventKind::Remove(_)
                    | notify::EventKind::Modify(notify::event::ModifyKind::Data(_))
            );
        }
        true
    })
}

impl BrowserWatches {
    pub(crate) fn contains(&self, project_id: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(project_id)
    }

    pub(crate) fn stop(&self, project_id: &str) {
        if let Some(active) = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(project_id)
        {
            active.task.abort();
        }
    }

    pub(crate) fn stop_all(&self) {
        let mut watches = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, active) in std::mem::take(&mut *watches) {
            active.task.abort();
        }
    }

    /// Return a note when the OS cannot watch every admitted directory.
    pub(crate) fn start(
        &self,
        shared: &std::sync::Arc<Shared>,
        project_id: &str,
        paths: Vec<PathBuf>,
    ) -> Option<String> {
        self.stop(project_id);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut watcher =
            match notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.as_ref().is_ok_and(changes_tree) {
                    let _ = tx.try_send(());
                }
            }) {
                Ok(watcher) => watcher,
                Err(error) => return Some(format!("File watching is unavailable: {error}")),
            };
        let mut count = 0;
        let mut failed = 0;
        for path in paths.into_iter().take(256) {
            if watcher.watch(&path, RecursiveMode::NonRecursive).is_ok() {
                count += 1;
            } else {
                failed += 1;
            }
        }
        if count == 0 {
            return Some("File watching is unavailable for this project.".into());
        }
        let weak = std::sync::Arc::downgrade(shared);
        let id = project_id.to_owned();
        let task = tokio::spawn(async move {
            let mut fallback = tokio::time::interval(Duration::from_secs(60));
            fallback.tick().await;
            loop {
                tokio::select! {
                    event = rx.recv() => {
                        if event.is_none() { break; }
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        while rx.try_recv().is_ok() {}
                    }
                    _ = fallback.tick() => {}
                }
                let Some(shared) = weak.upgrade() else { break };
                shared.emit(CoreEvent::GitBrowserChanged {
                    project_id: id.clone(),
                });
            }
        });
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                project_id.into(),
                ActiveWatch {
                    _watcher: watcher,
                    task,
                },
            );
        (failed > 0).then(|| {
            format!("File watching missed {failed} folders; the tree still refreshes periodically.")
        })
    }
}

impl Drop for BrowserWatches {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::changes_tree;
    use notify::event::{AccessKind, CreateKind, DataChange, ModifyKind, RemoveKind};
    use notify::{Event, EventKind};

    #[test]
    fn reads_and_locks_are_ignored_but_index_writes_refresh() {
        let source = std::path::PathBuf::from("/project/src/main.rs");
        let access = Event::new(EventKind::Access(AccessKind::Read)).add_path(source.clone());
        let edit = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(source);
        let index_read =
            Event::new(EventKind::Access(AccessKind::Read)).add_path("/project/.git/index".into());
        let index_write = Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
            .add_path("/project/.git/index".into());
        let index_create =
            Event::new(EventKind::Create(CreateKind::File)).add_path("/project/.git/index".into());
        let index_remove =
            Event::new(EventKind::Remove(RemoveKind::File)).add_path("/project/.git/index".into());
        let lock = Event::new(EventKind::Create(CreateKind::File))
            .add_path("/project/.git/index.lock".into());
        assert!(!changes_tree(&access));
        assert!(changes_tree(&edit));
        assert!(!changes_tree(&index_read));
        assert!(changes_tree(&index_write));
        assert!(changes_tree(&index_create));
        assert!(changes_tree(&index_remove));
        assert!(!changes_tree(&lock));
    }
}
