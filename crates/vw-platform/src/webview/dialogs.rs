//! Held native dialogs: thread-safe metadata for the tool and UI-thread continuations for each native engine.
//! Called only for agent-owned pages; ordinary user pages retain their native dialog UI.
//! Depends on: serde and synchronized native continuations.
//! Called by: native engine dialog adapters.
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};
use zeroize::Zeroizing;

/// Read-only facts about held dialogs, never prompt answers or authentication credentials.
#[derive(Default)]
pub struct Hub {
    next: AtomicU64,
    pending: Mutex<HashMap<String, Vec<Value>>>,
}

impl Hub {
    /// The first native dialog currently held for this exact mount.
    #[must_use]
    pub fn current(&self, label: &str) -> Option<Value> {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(label)
            .and_then(|v| v.first())
            .cloned()
    }
    fn open(&self, label: &str, mut info: Value) -> Option<String> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let rows = pending.entry(label.into()).or_default();
        if rows.len() >= 16 {
            return None;
        }
        let id = format!("dialog-{}", self.next.fetch_add(1, Ordering::Relaxed));
        info["dialogId"] = json!(id);
        info["id"] = json!(id);
        for field in ["message", "defaultPrompt", "url", "realm", "origin"] {
            if let Some(s) = info[field].as_str() {
                info[field] = json!(s.chars().take(4000).collect::<String>());
            }
        }
        rows.push(info);
        Some(id)
    }
    fn closed(&self, label: &str, id: &str) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(rows) = pending.get_mut(label) {
            rows.retain(|v| v["dialogId"] != id);
            if rows.is_empty() {
                pending.remove(label);
            }
        }
    }
}

/// A host-only answer; credential strings never enter tool arguments or receipts.
#[derive(Default)]
pub struct Answer {
    /// Accept versus dismiss.
    pub accept: bool,
    /// Prompt text, only for prompts.
    pub text: Option<String>,
    /// User name and password obtained through the app's vault consent route.
    pub credentials: Option<(Zeroizing<String>, Zeroizing<String>)>,
}

/// Live ownership is checked when a native dialog opens.
pub type Admit = Arc<dyn Fn() -> bool + Send + Sync>;
type Finish = Box<dyn FnOnce(Answer) -> Result<(), String>>;
thread_local! {static PENDING:RefCell<HashMap<(String,String),Finish>>=RefCell::new(HashMap::new());}

fn hold(hub: &Hub, label: &str, info: Value, finish: Finish) -> bool {
    let Some(id) = hub.open(label, info) else {
        let _ = finish(Answer::default());
        return true;
    };
    PENDING.with(|pending| pending.borrow_mut().insert((label.into(), id), finish));
    true
}

/// Answers exactly the observed mount/dialog on the native UI thread.
///
/// # Errors
/// The dialog was closed/replaced or its native completion failed.
pub fn answer(hub: &Hub, label: &str, id: &str, answer: Answer) -> Result<(), String> {
    let finish = PENDING
        .with(|pending| pending.borrow_mut().remove(&(label.into(), id.into())))
        .ok_or("The observed dialog is no longer open; read page.dialogs again.")?;
    let result = finish(answer);
    hub.closed(label, id);
    result
}

/// Releases held continuations when their native view closes. Must run on the native UI thread.
pub fn forget(hub: &Hub, label: &str) {
    release(hub, label);
    #[cfg(target_os = "macos")]
    macos::forget(label);
}

/// Dismisses held dialogs on ownership transfer without removing native delegates.
pub fn release(hub: &Hub, label: &str) {
    let ids: Vec<_> = PENDING.with(|pending| {
        pending
            .borrow()
            .keys()
            .filter(|(owner, _)| owner == label)
            .map(|(_, id)| id.clone())
            .collect()
    });
    for id in ids {
        let _ = answer(hub, label, &id, Answer::default());
    }
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::install;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{install, ownership};
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::install;
