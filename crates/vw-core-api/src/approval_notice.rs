//! Per-invocation observation of a presented approval, independent of permission grants.
//! Receipts scope preparation; the native confirmer and app card record only confirmed outcomes.
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use vw_ask::Outcome;
use vw_platform::sync::LockRecover;

#[derive(Clone, Default)]
pub(crate) struct Observation {
    pub(crate) approved: bool,
    refused: Option<Outcome>,
}

#[derive(Default)]
pub(crate) struct Notice(Mutex<Observation>);

impl Notice {
    /// Only the settled ask outcome attributes an Allow or No to this invocation.
    pub(crate) fn record(&self, outcome: &Outcome) {
        let mut notice = self.0.lock_or_recover();
        match outcome {
            Outcome::Allowed { .. } => notice.approved = true,
            Outcome::Refused { .. } => notice.refused = Some(outcome.clone()),
            _ => {}
        }
    }
}

impl Observation {
    /// Keep a confirmed refusal in ordinary MCP result metadata, separate from model-visible prose.
    pub(crate) fn refusal(&self, reason: String) -> Result<Value, String> {
        let Some(outcome) = &self.refused else {
            return Err(reason);
        };
        Ok(json!({"vwMcpResult": {
            "content": [{"type":"text", "text":reason}], "isError":false,
            "_meta":{"vwAskOutcome":outcome}
        }}))
    }
}

tokio::task_local! { static CURRENT: Arc<Notice>; }

pub(crate) async fn observe<T>(future: impl std::future::Future<Output = T>) -> (T, Observation) {
    let notice = Arc::new(Notice::default());
    let answer = CURRENT.scope(notice.clone(), future).await;
    let observation = notice.0.lock_or_recover().clone();
    (answer, observation)
}

pub(crate) fn capture() -> Option<Arc<Notice>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn settled(outcome: Outcome) -> Observation {
        let service = crate::ask::Service::new(
            Arc::new(crate::SystemAskClock::default()),
            tokio::sync::broadcast::channel(16).0,
        );
        let ((), observation) = observe(async {
            let mut handle = service.open_own(
                "tool/approval",
                Some("chat".into()),
                vw_ask::Requester::Agent,
                vec![vw_ask::Choice::Once, vw_ask::Choice::No],
                None,
            );
            handle.finish(outcome.clone());
            assert_eq!(handle.closed().await, outcome);
        })
        .await;
        observation
    }

    #[tokio::test]
    async fn every_own_card_keeps_its_confirmed_no_with_the_prepared_call() {
        let notice = settled(Outcome::Refused {
            by: vw_ask::AnsweredBy::Person,
            note: None,
        })
        .await;
        let result = notice
            .refusal("declined reason".into())
            .expect("confirmed No must survive every app card adapter");
        assert_eq!(
            result["vwMcpResult"]["_meta"]["vwAskOutcome"],
            json!({"type":"refused","by":"person","note":null})
        );
        assert!(!notice.approved);
    }

    #[tokio::test]
    async fn a_cancelled_own_card_does_not_become_a_person_no() {
        let notice = settled(Outcome::Cancelled {
            reason: "the turn was stopped".into(),
        })
        .await;
        assert_eq!(
            notice.refusal("stopped reason".into()),
            Err("stopped reason".into())
        );
        assert!(!notice.approved);
    }
}
