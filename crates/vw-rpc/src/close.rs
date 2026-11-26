//! Vibe's interrupt-before-close contract. Used by `Connection` and `Multiplexer` teardown, including Drop.
//! New work is fenced before draining admitted requests; uncertainty or a bounded failure kills the
//! process through its supervisor instead of parking a live turn on a closed 3.1.5 channel.
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tracing::warn;

use crate::connection::{Connection, Tasks};
use crate::error::DeathCause;
use crate::shared::{Shared, WriterCmd};

impl Connection {
    /// The task retains the exact channel and its writer across cancellation of the close caller.
    pub(crate) fn start_close(&self) {
        if !self.shared.begin_close() {
            return;
        }
        let conn = self.clone();
        tokio::spawn(async move {
            let limit = conn.shared.config.close_timeout;
            let stopped = tokio::time::timeout(limit, conn.stop_attached()).await;
            if !matches!(stopped, Ok(Ok(()))) {
                conn.shared.fail_close();
                conn.shared.close_done.send_replace(true);
                return;
            }
            if conn.shared.outbound.send(WriterCmd::Close).await.is_err() {
                conn.shared.fail_close();
            }
            conn.shared.close_done.send_replace(true);
            // Keep the channel writer alive until the server acknowledges close. A last-handle Drop
            // must not abort the writer between queueing Close and its delivery to the process.
            if tokio::time::timeout(limit, conn.closed()).await.is_err() {
                conn.shared.fail_close();
            }
        });
    }

    pub(crate) async fn wait_close_sent(&self) {
        let mut done = self.shared.close_done.subscribe();
        let _ = done.wait_for(|done| *done).await;
    }

    async fn stop_attached(&self) -> Result<(), ()> {
        let mut interrupted_turn = None;
        let mut interrupted_children = std::collections::BTreeSet::new();
        loop {
            let (pending, uncertain, session) = self.shared.close_state();
            if uncertain || self.shared.death().is_some() {
                return Err(());
            }
            let Some(session) = session else {
                if pending == 0 {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            };
            // Inspect a known attachment even while compaction or another admitted request is
            // pending. Its running turn must receive the interrupt before the drain budget expires.
            let state = self
                .close_request(
                    "session/read",
                    json!({
                        "sessionId":session, "history":{"limit":1}, "turns":{"limit":1}
                    }),
                )
                .await
                .map_err(|_| ())?;
            if state.pointer("/state/session/id").and_then(Value::as_str) != Some(&session) {
                return Err(());
            }
            let status = &state["state"]["session"]["status"];
            if matches!(
                status["type"].as_str(),
                Some("idle" | "failed" | "archived")
            ) {
                let children_quiet = self
                    .stop_children(&state["state"], &session, &mut interrupted_children)
                    .await?;
                let queue = &state["state"]["turnQueue"];
                if queue["items"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
                    && queue["paused"] != true
                {
                    return Err(());
                }
                let (remaining, uncertain, actual) = self.shared.close_state();
                // The final idle read must START after all admitted requests have answered; an
                // earlier idle snapshot could predate a late turn/start or a session hand-off.
                if children_quiet
                    && pending == 0
                    && remaining == 0
                    && !uncertain
                    && actual.as_deref() == Some(&session)
                {
                    return Ok(());
                }
            } else {
                let turn = status["activeTurnId"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or(())?;
                let target = (session.clone(), turn.to_owned());
                if interrupted_turn.as_ref() != Some(&target) {
                    self.interrupt_closing_turn(&session, turn).await?;
                    interrupted_turn = Some(target);
                }
                // An accepted interrupt can precede idle. Poll instead of interrupting that same
                // turn again; a genuinely new turn receives its own expected-id interrupt.
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn interrupt_closing_turn(&self, session: &str, turn: &str) -> Result<(), ()> {
        match self
            .close_request(
                "turn/interrupt",
                json!({
                    "sessionId":session, "expectedTurnId":turn
                }),
            )
            .await
        {
            Ok(ack) if ack["accepted"] == true => Ok(()),
            // Both supported versions return this exact response when the turn ends between
            // the live read and interrupt. The caller still requires a fresh idle read.
            Err(crate::RpcError::Remote { error, .. })
                if error.code == vw_protocol::ProtocolErrorCode::Conflict
                    && error.message == "No active turn" =>
            {
                Ok(())
            }
            _ => Err(()),
        }
    }

    async fn stop_children(
        &self,
        state: &Value,
        session: &str,
        interrupted: &mut std::collections::BTreeSet<String>,
    ) -> Result<bool, ()> {
        let mut active = std::collections::BTreeSet::new();
        for child in state["childSessions"].as_array().into_iter().flatten() {
            match child["status"]["type"].as_str() {
                Some("idle" | "failed" | "archived") => {}
                Some("running" | "blocked") => {
                    let id = child["id"].as_str().filter(|id| !id.is_empty()).ok_or(())?;
                    active.insert(id.to_owned());
                }
                _ => return Err(()),
            }
        }
        if active.is_empty() {
            return Ok(true);
        }
        if active.iter().any(|id| !interrupted.contains(id)) {
            let ack = self
                .close_request(
                    "session/subagents/interrupt",
                    json!({
                        "sessionId":session, "reason":"Chat connection closing"
                    }),
                )
                .await
                .map_err(|_| ())?;
            let results = ack["results"].as_array().ok_or(())?;
            for id in &active {
                if !results.iter().any(|result| {
                    result["sessionId"] == *id
                        && matches!(
                            result["outcome"].as_str(),
                            Some("interrupted" | "notRunning")
                        )
                }) {
                    return Err(());
                }
            }
            if results.iter().any(|result| {
                !matches!(
                    result["outcome"].as_str(),
                    Some("interrupted" | "notRunning")
                )
            }) {
                return Err(());
            }
            interrupted.extend(active);
        }
        Ok(false)
    }
}

/// Retains a stopped channel until its close receipt, even when the stop caller is cancelled.
pub(crate) fn acknowledged_stop(channel: Arc<Shared>) {
    let limit = channel.config.close_timeout;
    let Some(owner) = channel.owner() else {
        channel.mark_dead(
            DeathCause::Wedged {
                method: "session/stop".to_owned(),
                id: 0,
                after: limit,
            },
            None,
        );
        return;
    };
    // Start before delivering the stop reply, so cancelling its caller cannot abandon the bound.
    tokio::spawn(async move {
        let _owner = owner;
        let mut death = channel.death_watch();
        if !matches!(
            tokio::time::timeout(limit, death.wait_for(Option::is_some)).await,
            Ok(Ok(_))
        ) {
            channel.mark_dead(
                DeathCause::Wedged {
                    method: "session/stop".to_owned(),
                    id: 0,
                    after: limit,
                },
                None,
            );
        }
    });
}

/// Retains a dropped channel's writer and the process tasks until the same close contract finishes.
pub(crate) fn dropped_channel(
    channel: Arc<Shared>,
    own: Vec<tokio::task::JoinHandle<()>>,
    parent: Option<Arc<Tasks>>,
) {
    let tasks = Arc::new(Tasks::new(own, parent));
    if channel.death().is_none() && !channel.is_closing() {
        if tokio::runtime::Handle::try_current().is_err() {
            channel.fail_close();
            return;
        }
        let conn = Connection::from_parts(channel, tasks);
        conn.start_close();
    }
}

impl Shared {
    /// Marks the whole multiplexed process dead. The supervisor kills/reaps it on this signal;
    /// sending a channel close after an unsuccessful interrupt would leave its work parked.
    pub(crate) fn fail_close(&self) {
        // A concurrent close read can lose its channel after Vibe acknowledges session/stop.
        // That verified runtime stop has its own owned, bounded close-receipt watchdog.
        if self.stop_acknowledged()
            && self
                .death()
                .is_none_or(|report| report.cause == DeathCause::ClosedByClient)
        {
            return;
        }
        warn!(
            channel_id = self.channel_id,
            "could not verify stopped work before close; ending the server process"
        );
        let target = self.process.as_ref().and_then(std::sync::Weak::upgrade);
        target.as_deref().unwrap_or(self).mark_dead(
            DeathCause::Wedged {
                method: "interrupt-before-close".to_owned(),
                id: 0,
                after: self.config.close_timeout,
            },
            None,
        );
    }
}
