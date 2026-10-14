//! Stand-ins for the app's native pieces in synthetic tests: a scripted
//! confirmer (the native dialog) and a recording notifier (the OS notification).
//!
//! Neither can be driven from the Debug API: the test decides the answer (an atomic flag or a mode
//! file next to the host), the same way a human decides at the real dialog.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use vw_gate::{Confirmation, ConfirmationRequest};
use vw_host::SharedConfirmer;
use vw_platform::{NotificationRequest, Notifier, NotifyError};

/// A confirmer whose answer is a flag, with a log of what it was asked.
#[derive(Clone, Default)]
pub struct Scripted {
    approve: Arc<AtomicBool>,
    asked: Arc<Mutex<Vec<ConfirmationRequest>>>,
}

impl Scripted {
    /// Sets the answer for later requests.
    pub fn approve(&self, yes: bool) {
        self.approve.store(yes, Ordering::SeqCst);
    }
    /// Every request shown so far.
    pub fn asked(&self) -> Vec<ConfirmationRequest> {
        self.asked.lock().unwrap().clone()
    }
    /// The confirmer to give the host.
    pub fn confirmer(&self) -> SharedConfirmer {
        let me = self.clone();
        SharedConfirmer::new(move |request| {
            me.asked.lock().unwrap().push(request);
            let answer = if me.approve.load(Ordering::SeqCst) {
                Confirmation::Approved
            } else {
                Confirmation::Declined
            };
            std::future::ready(answer)
        })
    }
}

/// A confirmer driven by files in `dir`: `confirm.mode` containing `approve` approves, anything else
/// (or no file) declines; every request is appended to `confirm.log` as `method<TAB>summary`.
pub fn file_confirmer(dir: PathBuf) -> SharedConfirmer {
    SharedConfirmer::new(move |request: ConfirmationRequest| {
        use std::io::Write;
        let mode = std::fs::read_to_string(dir.join("confirm.mode")).unwrap_or_default();
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("confirm.log"))
        {
            let _ = writeln!(log, "{}\t{}", request.method, request.summary);
        }
        std::future::ready(if mode.trim() == "approve" {
            Confirmation::Approved
        } else {
            Confirmation::Declined
        })
    })
}

type Click = Box<dyn FnOnce() + Send>;
/// A shown notification and its click callback (taken when the test clicks it).
type Shown = (NotificationRequest, Option<Click>);

/// A notifier that records what it was asked to show and keeps the click callbacks.
#[derive(Clone, Default)]
pub struct Recording {
    shown: Arc<Mutex<Vec<Shown>>>,
    log: Option<PathBuf>,
}

impl Recording {
    /// Also appends each notification as a JSON line to `path`.
    pub fn logging_to(path: &Path) -> Self {
        Self {
            log: Some(path.to_path_buf()),
            ..Self::default()
        }
    }
    /// The notifications shown so far.
    pub fn shown(&self) -> Vec<NotificationRequest> {
        self.shown
            .lock()
            .unwrap()
            .iter()
            .map(|(r, _)| r.clone())
            .collect()
    }
    /// Simulates the user clicking notification `index`. False when it has no callback left.
    pub fn click(&self, index: usize) -> bool {
        let callback = self
            .shown
            .lock()
            .unwrap()
            .get_mut(index)
            .and_then(|(_, c)| c.take());
        callback.map(|c| c()).is_some()
    }
}

impl Notifier for Recording {
    fn show(&self, request: NotificationRequest, on_click: Click) -> Result<(), NotifyError> {
        if let Some(path) = &self.log {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(
                    file,
                    "{}",
                    serde_json::json!({"title": request.title, "body": request.body})
                );
            }
        }
        self.shown.lock().unwrap().push((request, Some(on_click)));
        Ok(())
    }
}
