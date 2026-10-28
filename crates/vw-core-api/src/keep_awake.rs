//! One app-wide idle-sleep hold, driven by the core's live session activity.
//! Depends on the platform backend, store and gate; the pump and callback events reconcile it.
//! The owning thread acquires/drops every OS handle; refusal is latched until work becomes idle.

use std::sync::{Arc, Mutex, PoisonError, mpsc};
use tokio::sync::{oneshot, watch};
use vw_platform::keep_awake::{IdleSleepBackend, IdleSleepHold};

use crate::core::Shared;

use crate::{KeepAwakeStatus as Status, KeepAwakeView};

enum Update {
    Working(bool),
    Enabled(bool, oneshot::Sender<()>),
    Stop,
}

pub(crate) struct AwakeState {
    send: mpsc::Sender<Update>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    pub status: watch::Receiver<KeepAwakeView>,
}

impl AwakeState {
    pub fn new(backend: Arc<dyn IdleSleepBackend>, enabled: bool) -> Self {
        let (send, receive) = mpsc::channel();
        let (status, view) = watch::channel(KeepAwakeView {
            enabled,
            status: Status::Idle,
        });
        let worker = std::thread::Builder::new()
            .name("idle-sleep-hold".to_owned())
            .spawn(move || run(backend.as_ref(), &receive, &status, enabled))
            .ok();
        Self {
            send,
            worker: Mutex::new(worker),
            status: view,
        }
    }

    pub fn view(&self) -> KeepAwakeView {
        *self.status.borrow()
    }

    pub fn working(&self, working: bool) {
        let _ = self.send.send(Update::Working(working));
    }

    pub async fn enabled(&self, enabled: bool) {
        let (reply, done) = oneshot::channel();
        if self.send.send(Update::Enabled(enabled, reply)).is_ok() {
            let _ = done.await;
        }
    }

    pub async fn shutdown(&self) {
        let _ = self.send.send(Update::Stop);
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

impl Drop for AwakeState {
    fn drop(&mut self) {
        let _ = self.send.send(Update::Stop);
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = worker.join();
        }
    }
}

fn run(
    backend: &dyn IdleSleepBackend,
    receive: &mpsc::Receiver<Update>,
    status: &watch::Sender<KeepAwakeView>,
    mut enabled: bool,
) {
    let (mut working, mut requested) = (false, false);
    let mut hold: Option<Box<dyn IdleSleepHold>> = None;
    for update in receive {
        let reply = match update {
            Update::Working(value) => {
                working = value;
                None
            }
            Update::Enabled(value, reply) => {
                enabled = value;
                Some(reply)
            }
            Update::Stop => break,
        };
        let desired = working && enabled;
        if desired != requested {
            requested = desired;
            let next = if desired {
                match backend.acquire() {
                    Ok(value) => {
                        hold = Some(value);
                        Status::Active
                    }
                    Err(_) => Status::Refused,
                }
            } else {
                hold.take();
                Status::Idle
            };
            status.send_modify(|view| view.status = next);
        }
        status.send_if_modified(|view| {
            if view.enabled == enabled {
                return false;
            }
            view.enabled = enabled;
            true
        });
        if let Some(reply) = reply {
            let _ = reply.send(());
        }
    }
    drop(hold);
    status.send_modify(|view| view.status = Status::Idle);
}

/// A parent turn includes its subagents. Child rows also cover work that outlives the parent turn.
pub(crate) fn state_working(state: &serde_json::Value, callback_pending: bool) -> bool {
    let status = |value: &serde_json::Value| {
        value
            .get("type")
            .unwrap_or(value)
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    let parent = status(&state["session"]["status"]);
    if state["childSessions"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|child| status(&child["status"]) == "running")
    {
        return true;
    }
    if callback_pending
        || parent == "blocked"
        || state["activeCallbacks"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    {
        return false;
    }
    parent == "running"
        || (!matches!(parent.as_str(), "failed" | "archived")
            && state["turns"]
                .as_array()
                .is_some_and(|turns| turns.iter().any(|turn| turn["status"] == "in_progress")))
}

pub(crate) fn reconcile(shared: &Shared) {
    let callbacks = shared.gate.open_callbacks();
    let working = shared.channels.open_channels().iter().any(|channel| {
        let pending = callbacks.iter().any(|callback| {
            callback.session_id == channel.session_id
                || shared.channels.callback_parent(&callback.id).as_deref()
                    == Some(&channel.session_id)
        });
        shared
            .store
            .view(&channel.session_id)
            .is_some_and(|view| state_working(&view.state, pending))
    });
    shared.keep_awake.working(working);
}

/// Forward only status changes; the worker holds no strong reference to the core.
pub(crate) fn publish(shared: &std::sync::Arc<Shared>) {
    let mut view = shared.keep_awake.status.clone();
    let weak = std::sync::Arc::downgrade(shared);
    tokio::spawn(async move {
        while view.changed().await.is_ok() {
            let current = *view.borrow_and_update();
            let Some(shared) = weak.upgrade() else {
                return;
            };
            shared.emit(crate::CoreEvent::KeepAwake { view: current });
        }
    });
}

pub(crate) fn from_config(config: &crate::CoreConfig) -> AwakeState {
    AwakeState::new(
        config.keep_awake_backend.clone(),
        config.secret_scan_data.as_ref().is_none_or(|data| {
            crate::app_settings::keep_awake_enabled(&data.join("settings.json"))
        }),
    )
}

impl crate::Core {
    /// Current preference and hold status without an OS request.
    #[must_use]
    pub fn keep_awake_view(&self) -> KeepAwakeView {
        self.shared.keep_awake.view()
    }

    pub(crate) async fn keep_awake(
        &self,
        request: crate::KeepAwakeRequest,
    ) -> Result<crate::CommandResult, crate::CoreError> {
        if let crate::KeepAwakeRequest::Write { enabled } = request {
            let path = self
                .shared
                .config
                .secret_scan_data
                .as_ref()
                .ok_or_else(|| {
                    crate::CoreError::new(
                        crate::ErrorCode::Internal,
                        "Could not save the keep awake setting.",
                    )
                })?
                .join("settings.json");
            crate::write_app_settings(
                &path,
                serde_json::Map::from_iter([("keepAwake".to_owned(), serde_json::json!(enabled))]),
            )
            .map_err(|_| {
                crate::CoreError::new(
                    crate::ErrorCode::Internal,
                    "Could not save the keep awake setting.",
                )
            })?;
            self.shared.keep_awake.enabled(enabled).await;
        }
        Ok(crate::CommandResult::KeepAwake {
            view: self.keep_awake_view(),
        })
    }
}
