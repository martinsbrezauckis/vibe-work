//! Background transcript hydration using the same backward history/list cursors as export.
//! Called after store adoption by sessions/pump; Channels cancels on close. Two shared loader slots,
//! bounded requests and generation-checked, byte-limited prepends preserve replacements and live events.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use vw_session::{HistoryRequest, SessionStore};

use crate::core::Shared;

struct Job {
    generation: u64,
    task: JoinHandle<()>,
}

/// Shares cancellation with the channel registry, including closes outside the dispatcher.
pub(crate) struct HistoryLoading {
    store: SessionStore,
    jobs: Mutex<HashMap<String, Job>>,
    slots: Arc<Semaphore>,
}

impl HistoryLoading {
    pub(crate) fn new(store: SessionStore) -> Self {
        Self {
            store,
            jobs: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(2)),
        }
    }

    pub(crate) fn start(&self, shared: &Arc<Shared>, id: &str) {
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        let request = self.store.history_request(id);
        if let Some(request) = &request
            && jobs
                .get(id)
                .is_some_and(|job| job.generation == request.generation)
        {
            return;
        }
        if let Some(job) = jobs.remove(id) {
            job.task.abort();
        }
        let Some(request) = request else {
            return;
        };
        let weak = Arc::downgrade(shared);
        let session_id = id.to_owned();
        jobs.insert(
            id.to_owned(),
            Job {
                generation: request.generation,
                task: tokio::spawn(load(weak, self.slots.clone(), session_id, request)),
            },
        );
    }

    pub(crate) fn stop(&self, id: &str) {
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        self.store.close_history(id);
        if let Some(job) = jobs.remove(id) {
            job.task.abort();
        }
    }

    pub(crate) fn stop_all(&self) {
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        for (id, job) in jobs.drain() {
            self.store.close_history(&id);
            job.task.abort();
        }
    }
}

impl Drop for HistoryLoading {
    fn drop(&mut self) {
        self.stop_all();
    }
}

struct Page {
    items: Vec<Value>,
    next: Option<String>,
}

async fn page(shared: &Shared, id: &str, request: &HistoryRequest) -> Result<Page, String> {
    // Give an interrupted send's resume/interrupt and verified restore priority over hydration.
    let _restore = shared.access.lock_restore(id).await;
    let mut reply = tokio::time::timeout(Duration::from_secs(30), shared.gate.call(
        "session/history/list",
        json!({"sessionId":id, "page":{"limit":request.limit,"direction":"backward","cursor":request.cursor}}),
        None,
    )).await.map_err(|_| "history read timed out".to_owned())?
        .map_err(|error| error.to_string())?.result;
    // Apply the same display projection used for loaded snapshots before entries reach subscribers.
    crate::provider_errors::value(&mut reply);
    crate::agent_tools::shell_commands::project(shared, &mut reply);
    #[cfg(feature = "video")]
    crate::video_projection::apply(shared, &mut reply);
    crate::edit_preview::project(shared, &mut reply);
    crate::chrome_control::approval::project(shared, &mut reply);
    let Value::Array(items) = reply["items"].take() else {
        return Err("history page has no items".into());
    };
    if items.iter().any(|item| item["id"].as_str().is_none()) {
        return Err("history page has an entry without an id".into());
    }
    let next = match reply.get("nextCursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor)) => Some(cursor.clone()),
        _ => return Err("history page has an invalid cursor".into()),
    };
    if next.as_deref() == Some(&request.cursor) || (items.is_empty() && next.is_some()) {
        return Err("history cursor did not advance".into());
    }
    Ok(Page { items, next })
}

async fn load(weak: Weak<Shared>, slots: Arc<Semaphore>, id: String, mut request: HistoryRequest) {
    // Wait without holding Shared or a page. Aborting a queued/active task drops its waiter/permit.
    // Hold the slot for the chain (including retries), so only two chats can build history at once.
    let Ok(_slot) = slots.acquire_owned().await else {
        return;
    };
    {
        let Some(shared) = weak.upgrade() else {
            return;
        };
        let Some(current) = shared.store.history_request(&id) else {
            return;
        };
        if current.generation != request.generation || current.cursor != request.cursor {
            return;
        }
        request = current;
    }
    let mut seen = HashSet::new();
    loop {
        let Some(shared) = weak.upgrade() else {
            return;
        };
        if !seen.insert(request.cursor.clone()) {
            break;
        }
        let mut result = page(&shared, &id, &request).await;
        if let Err(error) = &result {
            tracing::debug!(session_id = id, %error, "retrying older history read");
            tokio::time::sleep(Duration::from_millis(100)).await;
            if shared.store.history_request(&id).is_none_or(|current| {
                current.generation != request.generation || current.cursor != request.cursor
            }) {
                return;
            }
            result = page(&shared, &id, &request).await;
        }
        let page = match result {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(session_id = id, %error, "older history read stopped");
                return;
            }
        };
        if !shared
            .store
            .prepend_history(&id, &request, page.items, page.next)
        {
            return;
        }
        let Some(next) = shared.store.history_request(&id) else {
            return;
        };
        if next.generation != request.generation {
            return;
        }
        // A broken server can send fresh cursors with only duplicate entries. Valid small pages still
        // continue all the way to the entry cap, without an additional page-count limit.
        if next.loaded <= request.loaded {
            break;
        }
        request = next;
        // Give live notifications and foreground commands a turn between pages.
        tokio::task::yield_now().await;
    }
    tracing::warn!(
        session_id = id,
        "older history read stopped: cursor chain did not converge"
    );
}
