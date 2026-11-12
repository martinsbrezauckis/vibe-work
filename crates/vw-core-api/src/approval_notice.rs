//! Per-invocation observation of a presented approval, independent of permission grants.
//! Receipts scope preparation; the native confirmer and app card mark only an actual Allow.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

tokio::task_local! { static CURRENT: Arc<AtomicBool>; }

pub(crate) async fn observe<T>(future: impl std::future::Future<Output = T>) -> (T, bool) {
    let notice = Arc::new(AtomicBool::new(false));
    let answer = CURRENT.scope(notice.clone(), future).await;
    (answer, notice.load(Ordering::Relaxed))
}

pub(crate) fn capture() -> Option<Arc<AtomicBool>> {
    CURRENT.try_with(Clone::clone).ok()
}

/// Programmatic Vibe calls read structured results instead of the post-hook content blocks.
/// Keep the tool's fields and refusal reason, adding only the fact about this invocation.
pub(crate) fn result(
    outcome: Result<serde_json::Value, String>,
    approved: bool,
) -> Result<serde_json::Value, String> {
    if !approved {
        return outcome;
    }
    let note = vw_agent_tools::hook::ALLOWED_NOTE;
    match outcome {
        Ok(serde_json::Value::Object(mut fields)) => {
            fields.insert(
                "userApproval".into(),
                serde_json::Value::String(note.into()),
            );
            Ok(serde_json::Value::Object(fields))
        }
        Ok(serde_json::Value::String(text)) => {
            Ok(serde_json::Value::String(format!("{text}\n{note}")))
        }
        Ok(value) => Ok(serde_json::json!({"result":value, "userApproval":note})),
        Err(reason) => Err(format!("{reason}\n{note}")),
    }
}
