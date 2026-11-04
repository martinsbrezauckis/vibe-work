//! Deterministic saved-history transport for core paging tests. Answers the real read/history methods;
//! the gate, dispatcher, store and subscriptions run normally. No server, browser or model is launched.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use vw_rpc::RpcError;

pub(crate) struct FakeVibe {
    pub entries: Vec<Value>,
    pub calls: Mutex<Vec<Value>>,
    pub failures: AtomicUsize,
    pub overlap: AtomicBool,
    pub repeated_cursor: AtomicBool,
    pub hold: AtomicBool,
    pub release: tokio::sync::Semaphore,
    pub page_size: AtomicUsize,
    pub text_bytes: AtomicUsize,
    pub active: AtomicUsize,
    pub peak: AtomicUsize,
}

struct ActiveRequest<'a>(&'a AtomicUsize);

impl Drop for ActiveRequest<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl FakeVibe {
    pub fn new(count: usize) -> Arc<Self> {
        Arc::new(Self {
            entries: (0..count).map(|i| {
                if i == 0 || i == count - 1 {
                    json!({"id":format!("entry-{i}"), "type":"message", "role":"user",
                        "content":[{"type":"text", "text":if i == 0 {"Earliest browser request"} else {"Latest browser request"}}]})
                } else {
                    json!({"id":format!("entry-{i}"), "type":"notice", "level":"info", "message":"Browser step completed"})
                }
            }).collect(),
            calls: Mutex::new(Vec::new()),
            failures: AtomicUsize::new(0),
            overlap: AtomicBool::new(false),
            repeated_cursor: AtomicBool::new(false),
            hold: AtomicBool::new(false),
            release: tokio::sync::Semaphore::new(0),
            page_size: AtomicUsize::new(500),
            text_bytes: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        })
    }

    pub fn state(&self, id: &str) -> Value {
        let start = self.entries.len().saturating_sub(200);
        json!({"session":{"id":id,"harness":"unified", "createdAt":1000,"bumpedAt":2000,
            "status":{"type":"idle"}}, "eventId":0,"turns":[],
            "history":self.entries[start..], "historyBeforeCursor": (start > 0).then(|| start.to_string())})
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            "session/read" | "session/resume" => {
                Ok(json!({"state":self.state(params["sessionId"].as_str().unwrap())}))
            }
            "session/history/list" => {
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(active, Ordering::SeqCst);
                let _active = ActiveRequest(&self.active);
                self.calls.lock().unwrap().push(params.clone());
                if self.hold.load(Ordering::SeqCst) {
                    self.release.acquire().await.unwrap().forget();
                }
                // Leave a deterministic opportunity for a live entry or replacement between pages.
                tokio::time::sleep(Duration::from_millis(10)).await;
                if self
                    .failures
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok()
                {
                    return Err(RpcError::Timeout {
                        method: method.into(),
                        id: 1,
                        after: Duration::from_millis(10),
                    });
                }
                assert_eq!(params["page"]["direction"], "backward");
                let end = params["page"]["cursor"]
                    .as_str()
                    .map_or(self.entries.len(), |s| s.parse().unwrap());
                let limit = usize::try_from(params["page"]["limit"].as_u64().unwrap()).unwrap();
                assert!(limit <= 500);
                let start = end.saturating_sub(limit.min(self.page_size.load(Ordering::SeqCst)));
                let mut items = self.entries[start..end].to_vec();
                let text_bytes = self.text_bytes.load(Ordering::SeqCst);
                if text_bytes > 0 {
                    for item in &mut items {
                        item["message"] = Value::String("x".repeat(text_bytes));
                    }
                }
                if self.overlap.load(Ordering::SeqCst) && end < self.entries.len() {
                    items.push(self.entries[end].clone());
                }
                let next = if self.repeated_cursor.load(Ordering::SeqCst) {
                    Some(end.to_string())
                } else {
                    (start > 0).then(|| start.to_string())
                };
                Ok(json!({"items":items, "nextCursor":next}))
            }
            _ => panic!("unexpected fake Vibe method: {method} {params}"),
        }
    }
}

pub(crate) async fn rig(name: &str, count: usize) -> (crate::Core, Arc<FakeVibe>) {
    let core = crate::offline_core::start(name).await;
    let fake = FakeVibe::new(count);
    *core.shared.channels.test_transport.lock().unwrap() = Some(fake.clone());
    (core, fake)
}
