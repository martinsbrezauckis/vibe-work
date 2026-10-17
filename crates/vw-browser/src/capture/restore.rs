//! Owns a full-page capture's temporary page state across cancellation.
//! Depends on mounted script evaluation and tokio; called by shot before its first mutation.
//! The restore task outlives its caller and only touches the original mount and capture token.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use vw_preview::MountId;

use super::shot::Metrics;
use crate::{driver::Browser, scripts};

#[derive(Clone)]
pub(super) struct CapturePage {
    browser: Browser,
    mount: MountId,
    page: String,
    token: String,
}

impl CapturePage {
    pub(super) async fn new(browser: &Browser, session: &str) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let page = browser.page_of(session).await?;
        let mount = browser
            .service()
            .page_mount(session)
            .ok_or("the page closed before capture")?;
        Ok(Self {
            browser: browser.clone(),
            mount,
            page,
            token: NEXT.fetch_add(1, Ordering::Relaxed).to_string(),
        })
    }

    pub(super) async fn step(&self, op: &str, y: f64) -> Result<Metrics, String> {
        let answer: Metrics = self
            .browser
            .run_in_mount(
                &self.mount.session_id,
                scripts::fullpage_capture(&self.page, op, y, &self.token),
                Some(&self.mount),
            )
            .await?;
        self.browser
            .check_answer(&self.mount.session_id, &self.page, &answer.url)?;
        if let Some(error) = &answer.error {
            return Err(error.clone());
        }
        Ok(answer)
    }

    pub(super) async fn screenshot(&self) -> Result<Vec<u8>, String> {
        self.browser
            .service()
            .screenshot_png(&self.mount.session_id)
            .await
            .map_err(|e| e.to_string())
    }
}

pub(super) struct Restore {
    // Closing the channel, on either normal completion or Drop, releases the already-running restore task.
    done: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<(), String>>,
}

impl Restore {
    pub(super) fn start(page: CapturePage) -> Self {
        let (done, wait) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = wait.await;
            tokio::time::timeout(Duration::from_secs(15), page.step("restore", 0.0))
                .await
                .map_err(|_| "restoring the capture page took too long".to_owned())?
                .map(|_| ())
        });
        Self {
            done: Some(done),
            task,
        }
    }

    pub(super) async fn finish(mut self) -> Result<(), String> {
        self.done.take();
        (&mut self.task)
            .await
            .map_err(|e| format!("restoring the capture page: {e}"))?
    }
}
