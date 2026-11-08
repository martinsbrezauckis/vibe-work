//! Native guard prompts stay with their connected caller until answered.
//! Uses the shared confirmer; every invocation keeps its own one-use decision.
use crate::core::Shared;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};
use tokio::{sync::watch, task::AbortHandle};
use vw_gate::{Confirmation, ConfirmationRequest};
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
struct Pending {
    id: String,
    result: watch::Receiver<Option<Confirmation>>,
    task: AbortHandle,
    waiters: usize,
}
#[derive(Default)]
pub(crate) struct Calls(Mutex<HashMap<String, Pending>>);
impl Drop for Calls {
    fn drop(&mut self) {
        for pending in self
            .0
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            pending.task.abort();
        }
    }
}
struct Owner<'a> {
    calls: &'a Calls,
    key: String,
    id: String,
    cancel: bool,
}
impl Drop for Owner<'_> {
    fn drop(&mut self) {
        if !self.cancel {
            return;
        }
        let mut calls = self.calls.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(call) = calls.get_mut(&self.key).filter(|call| call.id == self.id) {
            call.waiters = call.waiters.saturating_sub(1);
            if call.waiters == 0 {
                call.task.abort();
                calls.remove(&self.key);
            }
        }
    }
}
fn pending(
    shared: &Arc<Shared>,
    mut request: ConfirmationRequest,
) -> Result<(String, String, watch::Receiver<Option<Confirmation>>), String> {
    let mut key = format!("{}:{}", request.method, request.params);
    let (id, result) = {
        let mut calls = shared
            .guard_asks
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Concurrent calls must never reuse one native Allow, even when their text is identical.
        if calls.contains_key(&key) {
            key = format!(
                "{key}:concurrent:{}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
        }
        if let Some(pending) = calls.get_mut(&key) {
            pending.waiters += 1;
            (pending.id.clone(), pending.result.clone())
        } else {
            if calls.len() >= 500 {
                return Err("Too many open permission prompts.".into());
            }
            let id = format!(
                "guard-{}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            request.params["askId"] = serde_json::Value::String(id.clone());
            let presentation = shared.confirmer.begin(request);
            let (send, result) = watch::channel(None);
            // A No is handed to the waiting caller, which returns the declined result; the turn goes on.
            let task = tokio::spawn(async move {
                let answer = presentation.await;
                send.send_replace(Some(answer));
            });
            calls.insert(
                key.clone(),
                Pending {
                    id: id.clone(),
                    result: result.clone(),
                    task: task.abort_handle(),
                    waiters: 1,
                },
            );
            (id, result)
        }
    };
    Ok((key, id, result))
}

/// Keeps the native prompt open until the caller receives its answer or disconnects.
pub(crate) async fn wait(
    shared: &Arc<Shared>,
    request: ConfirmationRequest,
    what: &str,
) -> Result<(), String> {
    decision(shared, request, what, false).await.map(|_| ())
}

/// Browser callers retain the structured No for their action logs and receipts.
pub(crate) async fn answer(
    shared: &Arc<Shared>,
    request: ConfirmationRequest,
    what: &str,
) -> Result<(Confirmation, u64), String> {
    decision(shared, request, what, true).await
}

async fn decision(
    shared: &Arc<Shared>,
    request: ConfirmationRequest,
    what: &str,
    include_refusal: bool,
) -> Result<(Confirmation, u64), String> {
    let (key, id, mut result) = pending(shared, request)?;
    let mut owner = Owner {
        calls: &shared.guard_asks,
        key,
        id: id.clone(),
        cancel: true,
    };
    result
        .wait_for(Option::is_some)
        .await
        .map_err(|_| "Permission prompt closed.".to_owned())?;
    let answer = (*result.borrow()).unwrap_or(Confirmation::Declined);
    {
        let mut calls = shared
            .guard_asks
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // The first waiter to wake closes this sheet's entry; a newer sheet for the same exact command keeps its own.
        if calls.get(&owner.key).is_some_and(|p| p.id == id) {
            calls.remove(&owner.key);
        }
    }
    owner.cancel = false;
    // Only this invocation receives the native decision.
    if answer.approved() || include_refusal {
        let record = shared.asks.outcome(&id);
        Ok((
            answer,
            record.map_or(shared.asks.now_ms(), |r| r.ask.opened_at),
        ))
    } else {
        Err(shared.asks.outcome(&id).map_or_else(
            || answer.agent_text(what),
            |r| vw_ask::agent_text(&r.outcome, what),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simultaneous_git_calls_have_separate_decisions() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let request = |call| ConfirmationRequest {
            method: "vibe-work/gitGuard".into(),
            requester: vw_gate::Requester::Agent,
            summary: "git reset --hard HEAD~1".into(),
            params: serde_json::json!({"sessionId":"s", "cwd":"/project",
                "command":"git reset --hard HEAD~1", "toolCallId":call}),
            always: None,
        };
        let first = pending(&fixture.core.shared, request("first")).unwrap();
        let second = pending(&fixture.core.shared, request("second")).unwrap();
        // Abort before polling the server-free fixture's native presenter.
        for call in fixture.core.shared.guard_asks.0.lock().unwrap().values() {
            call.task.abort();
        }
        assert_ne!(first.1, second.1, "one Allow must cover only one tool call");
        fixture.finish().await;
    }
}
