//! Bounded start/restart replies for the agent-tools endpoint. Depends on tokio, `serde_json` and the
//! sidecar's wire budget; called by `tools`. A blocking OS scan can continue after the reply, but a
//! recorded spawn is acknowledged with its pid instead of becoming a transport timeout or refusal.

use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;
use tokio::task::{JoinError, JoinHandle};
use vw_agent_tools::wire;

/// Leaves ten seconds for the sidecar's connection/write/read and core answer serialization.
const REPLY_BUDGET: Duration = wire::CALL_TIMEOUT.saturating_sub(Duration::from_secs(10));

pub(crate) async fn finish(
    job: JoinHandle<Result<Value, String>>,
    spawned: oneshot::Receiver<Value>,
    start_wait: Duration,
    pending: Value,
) -> Result<Value, String> {
    finish_with_budget(job, spawned, start_wait, pending, REPLY_BUDGET).await
}

async fn finish_with_budget(
    mut job: JoinHandle<Result<Value, String>>,
    spawned: oneshot::Receiver<Value>,
    start_wait: Duration,
    mut pending: Value,
    budget: Duration,
) -> Result<Value, String> {
    let outcome = tokio::time::timeout(budget, async {
        tokio::select! {
            result = &mut job => joined(result),
            recorded = spawned => {
                if let Ok(recorded) = recorded {
                    pending = recorded;
                    match tokio::time::timeout(start_wait.min(budget), &mut job).await {
                        Ok(result) => match result {
                            Ok(result) => result,
                            Err(error) => {
                                pending["readinessError"] = Value::String(format!("readiness check failed: {error}"));
                                Ok(pending.clone())
                            }
                        },
                        Err(_) => Ok(pending.clone()),
                    }
                } else {
                    joined(job.await)
                }
            }
        }
    }).await;
    // Dropping a blocking job's handle leaves it running: never cancel a process mutation or start
    // another one as recovery. The registry/launch record and processes_list supply the next result.
    outcome.unwrap_or(Ok(pending))
}

fn joined(result: Result<Result<Value, String>, JoinError>) -> Result<Value, String> {
    result.map_err(|error| format!(
        "{} the process request failed before its result was confirmed: {error}; call processes_list before repeating it",
        wire::UNKNOWN_OUTCOME,
    ))?
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn stalled_scan_after_spawn_returns_the_recorded_pid() {
        let (spawned, received) = oneshot::channel();
        let (release, blocked) = oneshot::channel();
        let job = tokio::spawn(async move {
            spawned
                .send(json!({"pid": 42, "status": "running (not listening yet)"}))
                .unwrap();
            blocked.await.unwrap();
            Ok(json!({"pid": 42, "status": "running"}))
        });
        let result = finish_with_budget(
            job,
            received,
            Duration::from_secs(60),
            json!({}),
            Duration::from_millis(10),
        )
        .await
        .unwrap();
        assert_eq!(result["pid"], 42);
        assert_eq!(result["status"], "running (not listening yet)");
        release.send(()).unwrap();
    }

    #[tokio::test]
    async fn slow_preflight_reports_pending_instead_of_not_run() {
        let (spawned, received) = oneshot::channel();
        let (release, blocked) = oneshot::channel();
        let job = tokio::spawn(async move {
            blocked.await.unwrap();
            spawned.send(json!({"pid": 42})).ok();
            Ok(json!({"pid": 42}))
        });
        let result = finish_with_budget(
            job,
            received,
            Duration::from_secs(8),
            json!({"status": "start/restart pending"}),
            Duration::from_millis(10),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "start/restart pending");
        release.send(()).unwrap();
    }

    #[tokio::test]
    async fn a_real_pre_spawn_failure_keeps_its_reason() {
        let (spawned, received) = oneshot::channel();
        let job = tokio::spawn(async move {
            drop(spawned);
            Err("the command could not be started: folder missing".to_owned())
        });
        let result = finish(job, received, Duration::from_secs(8), json!({})).await;
        assert_eq!(
            result.unwrap_err(),
            "the command could not be started: folder missing"
        );
    }
}
