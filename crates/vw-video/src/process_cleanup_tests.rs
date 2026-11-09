//! Cancellation while final reaping is pending must retain staging; uses a deterministic process-wait gate.
//! Complements the real copied-process tests without relying on Windows termination timing.
use super::*;
use std::{future::Future, io, pin::Pin, process::ExitStatus, sync::Arc};
use tokio::sync::Notify;

#[derive(Debug)]
struct ReapGate {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

impl ChildWrapper for ReapGate {
    fn inner(&self) -> &dyn ChildWrapper {
        self
    }
    fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
        self
    }
    fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
        self
    }
    fn start_kill(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<ExitStatus>> + Send + '_>> {
        Box::pin(async {
            self.started.notify_one();
            self.release.notified().await;
            Ok(ExitStatus::default())
        })
    }
}

#[tokio::test]
async fn cancellation_during_final_reap_keeps_the_directory_until_reaping_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_path_buf();
    std::fs::write(path.join("worker-copy"), b"helper").unwrap();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut child = Child {
        process: Some(Box::new(ReapGate {
            started: started.clone(),
            release: release.clone(),
        })),
        directory: Some(crate::staging::Directory(directory)),
    };
    let task = tokio::spawn(async move {
        reap(&mut child).await;
    });
    tokio::time::timeout(Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    task.abort();
    let _ = task.await;
    assert!(
        path.exists(),
        "staging disappeared before the cancelled final wait reaped the worker"
    );
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), async {
        while path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
