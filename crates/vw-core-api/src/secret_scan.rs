//! Secret scan lifecycle, scheduling and typed commands. Values and live reports stay on one worker thread.
//! Depends on the key service, app settings, existing Vibe config resolver and session leases; called by core startup/dispatch.

use crate::core::Shared;
use crate::{
    Command, CommandResult, Core, CoreError, CoreEvent, ErrorCode, Origin, SecretScanRequest,
    SecretScanStatus, SecretScanView,
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{Notify, oneshot},
    task::JoinHandle,
};

pub(crate) const START_IDLE: Duration = Duration::from_secs(30);

pub(crate) enum Job {
    VaultChanged,
    Scan {
        folder: PathBuf,
        full: bool,
    },
    Redact {
        finding_id: String,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    Stop,
}

pub(crate) struct ScanState {
    pub view: Mutex<SecretScanView>,
    pub errors: Mutex<Vec<PathBuf>>,
    pub activity: Mutex<Instant>,
    pub loaded: AtomicBool,
    pub stopped: AtomicBool,
    pub pending: AtomicBool,
    pub full: AtomicBool,
    pub running: AtomicBool,
    pub wake: Notify,
    sender: Mutex<Option<std::sync::mpsc::Sender<Job>>>,
    task: Mutex<Option<JoinHandle<()>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl ScanState {
    pub fn new(data: Option<&std::path::Path>) -> Self {
        let settings = data
            .map(|data| crate::app_settings::scan_settings(&data.join("settings.json")))
            .unwrap_or_default();
        let view = SecretScanView {
            settings,
            status: if settings.enabled {
                SecretScanStatus::Idle
            } else {
                SecretScanStatus::Off
            },
            ..SecretScanView::default()
        };
        Self {
            view: Mutex::new(view),
            errors: Mutex::new(Vec::new()),
            activity: Mutex::new(Instant::now()),
            loaded: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            pending: AtomicBool::new(settings.enabled),
            full: AtomicBool::new(false),
            running: AtomicBool::new(false),
            wake: Notify::new(),
            sender: Mutex::new(None),
            task: Mutex::new(None),
            worker: Mutex::new(None),
        }
    }
    pub fn view(&self) -> SecretScanView {
        self.view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    pub fn file_errors(&self) -> Vec<String> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|path| {
                let text = path.to_string_lossy();
                vw_secrets::scrub(text.as_bytes()).map_or_else(
                    || text.into_owned(),
                    |(bytes, _)| String::from_utf8_lossy(&bytes).into_owned(),
                )
            })
            .collect()
    }
    pub fn activity(&self) {
        *self.activity.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
        self.wake.notify_one();
    }
    pub fn idle(&self) -> Duration {
        self.activity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed()
    }
    pub fn loaded(&self) {
        self.loaded.store(true, Ordering::Release);
        self.activity();
    }
    pub fn send(&self, job: Job) -> Result<(), CoreError> {
        self.sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .ok_or_else(unavailable)?
            .send(job)
            .map_err(|_| unavailable())
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.pending.store(false, Ordering::Release);
        if let Some(task) = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
        let _ = self.send(Job::Stop);
        self.wake.notify_one();
    }

    pub async fn shutdown(&self) {
        self.stop();
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(worker) = worker {
            let _ = tokio::task::spawn_blocking(move || worker.join()).await;
        }
    }
}

impl Drop for ScanState {
    fn drop(&mut self) {
        self.stop();
    }
}

fn unavailable() -> CoreError {
    CoreError::new(
        ErrorCode::Internal,
        "The session log checker is unavailable.",
    )
}

pub(crate) fn publish(shared: &Shared) {
    shared.emit(CoreEvent::SecretScan {
        view: shared.secret_scan.view(),
    });
}

pub(crate) fn status(shared: &Shared, status: SecretScanStatus) {
    let mut view = shared
        .secret_scan
        .view
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    view.status = if view.settings.enabled {
        status
    } else {
        SecretScanStatus::Off
    };
    drop(view);
    publish(shared);
}

pub(crate) fn busy(shared: &Shared) -> bool {
    shared.store.views().iter().any(|view| {
        view.state["turns"]
            .as_array()
            .is_some_and(|turns| turns.iter().any(|turn| turn["status"] == "in_progress"))
    })
}

pub(crate) fn start(shared: &Arc<Shared>) {
    if shared.config.secret_scan_data.is_none() {
        return;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let weak = Arc::downgrade(shared);
    let runtime = tokio::runtime::Handle::current();
    if let Ok(worker) = std::thread::Builder::new()
        .name("session-log-checker".to_owned())
        .spawn(move || crate::secret_scan_worker::run(&weak, &runtime, receiver))
    {
        *shared
            .secret_scan
            .sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(sender);
        *shared
            .secret_scan
            .worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(worker);
    } else {
        status(shared, SecretScanStatus::Error);
        return;
    }
    let weak = Arc::downgrade(shared);
    let task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let state = &shared.secret_scan;
            if state.stopped.load(Ordering::Acquire) {
                return;
            }
            let view = state.view();
            let immediate = state.full.load(Ordering::Acquire);
            if !view.settings.enabled
                || !state.pending.load(Ordering::Acquire)
                || state.running.load(Ordering::Acquire)
                || !state.loaded.load(Ordering::Acquire)
                || !ready(
                    state.loaded.load(Ordering::Acquire),
                    state.idle(),
                    busy(&shared),
                    immediate,
                )
            {
                continue;
            }
            state.pending.store(false, Ordering::Release);
            state.running.store(true, Ordering::Release);
            let full = state.full.swap(false, Ordering::AcqRel);
            status(&shared, SecretScanStatus::Checking);
            let core = Core {
                shared: shared.clone(),
            };
            if let Ok(folder) = resolve_folder(&core).await {
                if state.send(Job::Scan { folder, full }).is_err() {
                    state.running.store(false, Ordering::Release);
                    status(&shared, SecretScanStatus::Error);
                }
            } else {
                state.running.store(false, Ordering::Release);
                status(&shared, SecretScanStatus::Error);
            }
        }
    });
    *shared
        .secret_scan
        .task
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(task);
}

pub(crate) fn ready(loaded: bool, idle: Duration, busy: bool, immediate: bool) -> bool {
    loaded && (immediate || (!busy && idle >= START_IDLE))
}

/// Vibe resolves `save_dir` (defaults, overrides, expanduser and relative paths). Use that effective result.
pub(crate) async fn resolve_folder(core: &Core) -> Result<PathBuf, CoreError> {
    let id = core
        .start_helper_chat(Origin::Ui, "Session logs could not be checked")
        .await?;
    let reply = core
        .shared
        .gate
        .call("config/fields/read", json!({"sessionId": id}), None)
        .await;
    let _ = core.end_helper_chat(&id).await;
    let reply = reply.map_err(|error| CoreError::from_gate("config/fields/read", &error))?;
    reply
        .result
        .get("fields")
        .and_then(serde_json::Value::as_array)
        .and_then(|fields| {
            fields.iter().find(|field| {
                field.get("name").and_then(serde_json::Value::as_str) == Some("session_logging")
            })
        })
        .and_then(|field| field.pointer("/value/save_dir"))
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .filter(|folder| folder.is_absolute())
        .ok_or_else(unavailable)
}

impl Core {
    pub(crate) async fn secret_scan(
        &self,
        request: SecretScanRequest,
    ) -> Result<CommandResult, CoreError> {
        let state = &self.shared.secret_scan;
        match request {
            SecretScanRequest::Read {} => {}
            SecretScanRequest::Activity {} => {
                state.activity();
                return Ok(CommandResult::Reply {
                    result: serde_json::Value::Null,
                    client_user_message_id: None,
                    user_entry_ids: Vec::new(),
                });
            }
            SecretScanRequest::Write { settings } => {
                let data = self
                    .shared
                    .config
                    .secret_scan_data
                    .as_ref()
                    .ok_or_else(unavailable)?;
                crate::write_app_settings(
                    &data.join("settings.json"),
                    serde_json::Map::from_iter([("secretScan".to_owned(), json!(settings))]),
                )
                .map_err(|_| {
                    CoreError::new(
                        ErrorCode::Internal,
                        "Could not save the session log settings.",
                    )
                })?;
                let saved = crate::app_settings::scan_settings(&data.join("settings.json"));
                let mut view = state.view.lock().unwrap_or_else(PoisonError::into_inner);
                let first = !view.settings.enabled && saved.enabled;
                view.settings = saved;
                if !saved.enabled {
                    view.status = SecretScanStatus::Off;
                    state.pending.store(false, Ordering::Release);
                }
                if first {
                    state.pending.store(true, Ordering::Release);
                    state.full.store(true, Ordering::Release);
                    view.status = SecretScanStatus::Checking;
                }
                drop(view);
                publish(&self.shared);
            }
            SecretScanRequest::CheckNow {} => {
                if !state.view().settings.enabled {
                    return Err(CoreError::new(
                        ErrorCode::InvalidParams,
                        "Turn on session log scanning first.",
                    ));
                }
                state.pending.store(true, Ordering::Release);
                state.full.store(true, Ordering::Release);
                status(&self.shared, SecretScanStatus::Checking);
            }
            SecretScanRequest::Redact { finding_id } => {
                let (reply, done) = oneshot::channel();
                state.send(Job::Redact { finding_id, reply })?;
                done.await.map_err(|_| unavailable())??;
            }
        }
        Ok(CommandResult::SecretScan {
            view: state.view(),
            file_errors: state.file_errors(),
        })
    }
}

pub(crate) fn note_command(shared: &Shared, command: &Command) {
    if !matches!(
        command,
        Command::ReadStatus {}
            | Command::ReadSession { .. }
            | Command::ReadAnomalies { .. }
            | Command::ListCallbacks {}
            | Command::SecretScan {
                request: SecretScanRequest::Read {}
            }
    ) {
        shared.secret_scan.activity();
    }
}
