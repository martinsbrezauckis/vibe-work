//! Bounded native request observations, owned by one webview and discarded on its next document.
//! Engine adapters record facts only; browser tools apply Full-access header checks and output redaction.
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use tauri::{Webview, Wry};
use vw_preview::EngineError;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(windows)]
mod windows;
pub(crate) type Shared = Arc<Mutex<Log>>;
#[derive(Default)]
pub(crate) struct Log {
    rows: VecDeque<Value>,
    bytes: usize,
    prefix: String,
    next: u64,
    dropped: usize,
    unavailable: Option<String>,
}
pub(super) fn lock(log: &Shared) -> MutexGuard<'_, Log> {
    log.lock().unwrap_or_else(PoisonError::into_inner)
}
impl Log {
    pub(super) fn reset(&mut self) {
        self.rows.clear();
        self.bytes = 0;
        self.dropped = 0;
    }
    pub(super) fn start(&mut self, mut row: Value) -> String {
        self.next += 1;
        let id = format!("req-{}-{}", self.prefix, self.next);
        row["requestId"] = json!(id);
        self.bytes += row.to_string().len();
        self.rows.push_back(row);
        self.trim();
        id
    }
    pub(super) fn update(&mut self, id: &str, fields: Value) {
        if let Some(row) = self.rows.iter_mut().find(|r| r["requestId"] == id)
            && let (Some(row), Value::Object(fields)) = (row.as_object_mut(), fields)
        {
            self.bytes = self
                .bytes
                .saturating_sub(serde_json::to_vec(&row).map_or(0, |v| v.len()));
            row.extend(fields);
            self.bytes += serde_json::to_vec(&row).map_or(0, |v| v.len());
        }
        self.trim();
    }
    fn trim(&mut self) {
        while self.rows.len() > 256 || self.bytes > 2 * 1024 * 1024 {
            if let Some(row) = self.rows.pop_front() {
                self.bytes = self.bytes.saturating_sub(row.to_string().len());
            }
            self.dropped += 1;
        }
    }
    pub(crate) fn snapshot(&self) -> Result<Value, EngineError> {
        if let Some(reason) = &self.unavailable {
            return Err(EngineError::Unsupported(reason.clone()));
        }
        Ok(
            json!({"requests":self.rows,"dropped":self.dropped,"coverage":"Native requests observed in this webview since document load; redirect chains and cross-process child requests may be incomplete."}),
        )
    }
}
pub(crate) async fn install(view: &Webview<Wry>) -> Shared {
    let log = Arc::new(Mutex::new(Log {
        prefix: vw_preview::accessor_name().replace("__vw_console_", ""),
        ..Log::default()
    }));
    #[cfg(any(target_os = "linux", windows, target_os = "macos"))]
    let result = {
        let records = log.clone();
        crate::platform::with_native(
            view,
            "watching page requests",
            std::time::Duration::from_secs(5),
            move |native, reply| {
                #[cfg(target_os = "linux")]
                {
                    linux::install(&native.inner(), records);
                    let _ = reply.send(Ok(()));
                }
                #[cfg(target_os = "macos")]
                {
                    let _ = reply.send(macos::install(native.inner(), records));
                }
                #[cfg(windows)]
                windows::install(
                    &native.controller(),
                    records,
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
            },
        )
        .await
    };
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    let result: Result<(), EngineError> = {
        let _ = view;
        Err(EngineError::Unsupported("This native engine has no public complete request/header event API; use page.errors for resource failures or inspect the page in its developer tools.".into()))
    };
    if let Err(error) = result {
        lock(&log).unavailable = Some(error.to_string());
    }
    log
}
