//! Bounded secret status waits and single-delivery receipts. Uses the broker's command state;
//! called by tool dispatch. Receipts keep outcome/time, never output, until the completion retention ends.
use super::{AgentTools, Run, RunState, Value, WAIT, json, terminal};

const NOT_FOUND: &str =
    "secret_run request not found in this session (completed requests are kept 15 minutes)";

/// Waits up to WAIT for a running command to finish.
pub(super) async fn wait(tools: &AgentTools, session_id: &str, id: &str) -> Result<Value, String> {
    tools.secret_runs.open_of(session_id);
    let mut receiver = {
        let runs = tools.secret_runs.lock();
        runs.get(id)
            .filter(|run| run.session_id == session_id)
            .ok_or(NOT_FOUND)?
            .state
            .clone()
    };
    let _ = tokio::time::timeout(WAIT, receiver.wait_for(terminal)).await;
    // Re-read under the lock: concurrent readers may have collected the same watched outcome.
    tools.secret_runs.open_of(session_id);
    let mut runs = tools.secret_runs.lock();
    let run = runs
        .get_mut(id)
        .filter(|run| run.session_id == session_id)
        .ok_or(NOT_FOUND)?;
    if let Some(result) = collect(run) {
        return result;
    }
    run.handed_off = true;
    Ok(json!({
        "status": "running",
        "requestId": id,
    }))
}

/// Replace a collected payload with a small receipt, while retaining its original expiry.
fn collect(run: &mut Run) -> Option<Result<Value, String>> {
    let state = run.state.borrow().clone();
    let (outcome, finished, result) = match state {
        RunState::Finished(result, finished) => ("finished", finished, Ok(result)),
        RunState::Refused(reason, finished) => ("refused", finished, Err(reason)),
        RunState::Reported { outcome, at, .. } => {
            return Some(Ok(json!({
                "status": format!("This secret_run was {outcome}; its outcome was already reported at {at}."),
                "next": "Use the outcome already reported; do not repeat secret_run for this request.",
            })));
        }
        RunState::Running => return None,
    };
    let at = chrono::Local::now().format("%H:%M").to_string();
    run.state = tokio::sync::watch::channel(RunState::Reported {
        outcome,
        at,
        finished,
    })
    .1;
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::super::{KEEP, Run, RunState, SecretRuns};
    use std::time::Instant;

    #[test]
    fn secret_results_get_their_full_collection_window_after_completion() {
        let runs = SecretRuns::default();
        let now = Instant::now();
        for (id, state) in [
            ("running", RunState::Running),
            ("fresh", RunState::Finished(serde_json::Value::Null, now)),
            (
                "old",
                RunState::Finished(serde_json::Value::Null, now.checked_sub(KEEP).unwrap()),
            ),
            ("refused", RunState::Refused("No".into(), now)),
            (
                "reported",
                RunState::Reported {
                    outcome: "finished",
                    at: "12:34".into(),
                    finished: now,
                },
            ),
            (
                "old-reported",
                RunState::Reported {
                    outcome: "refused",
                    at: "12:34".into(),
                    finished: now.checked_sub(KEEP).unwrap(),
                },
            ),
            (
                "old-refused",
                RunState::Refused("No".into(), now.checked_sub(KEEP).unwrap()),
            ),
        ] {
            let (_, state) = tokio::sync::watch::channel(state);
            runs.lock().insert(
                id.into(),
                Run {
                    session_id: "s".into(),
                    state,
                    handed_off: false,
                },
            );
        }
        assert_eq!(runs.open_of("s"), 1);
        let kept = runs.lock();
        assert_eq!(kept.len(), 4);
        for id in ["running", "fresh", "refused", "reported"] {
            assert!(kept.contains_key(id));
        }
    }

    #[test]
    fn a_receipt_drops_output_keeps_its_time_and_does_not_extend_retention() {
        let finished = Instant::now().checked_sub(KEEP / 2).unwrap();
        for (state, outcome) in [
            (
                RunState::Finished(serde_json::json!({"stdout":"one delivery"}), finished),
                "finished",
            ),
            (RunState::Refused("No".into(), finished), "refused"),
        ] {
            let mut run = Run {
                session_id: "s".into(),
                state: tokio::sync::watch::channel(state).1,
                handed_off: false,
            };
            let first = super::collect(&mut run).unwrap();
            assert_eq!(first.is_err(), outcome == "refused");
            let second = super::collect(&mut run).unwrap().unwrap();
            let third = super::collect(&mut run).unwrap().unwrap();
            assert_eq!(second, third);
            let state = run.state.borrow();
            let RunState::Reported {
                outcome: reported,
                at,
                finished: kept,
            } = &*state
            else {
                panic!("payload was not discarded")
            };
            assert_eq!(*reported, outcome);
            assert_eq!(*kept, finished);
            assert_eq!(at.len(), 5);
            assert_eq!(at.as_bytes()[2], b':');
            assert_eq!(
                second["status"],
                format!("This secret_run was {outcome}; its outcome was already reported at {at}.")
            );
            assert!(second.get("requestId").is_none());
            assert!(!second.to_string().contains("one delivery"));
        }
    }
}
