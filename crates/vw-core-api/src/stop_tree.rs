//! Stop ownership and native callback cancellation. Uses childSessions and the live callback channel, never a
//! fork's parentSessionId. Called by stops/dispatch; Vibe owns child runtimes on the parent's channel.
use crate::{core::Shared, error::CoreError};
use serde_json::json;
use std::collections::BTreeSet;

/// Cached ownership, including callbacks that arrived before their child summary.
pub(crate) fn family(shared: &Shared, root: &str) -> Vec<String> {
    let mut seen = BTreeSet::from([root.to_owned()]);
    let mut pending = vec![root.to_owned()];
    while let Some(id) = pending.pop() {
        if let Some(view) = shared.store.view(&id) {
            for child in view.state["childSessions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["id"].as_str())
            {
                if seen.insert(child.into()) {
                    pending.push(child.into());
                }
            }
        }
        for card in shared.gate.open_callbacks() {
            if shared.channels.callback_parent(&card.id).as_deref() == Some(&id)
                && seen.insert(card.session_id.clone())
            {
                pending.push(card.session_id);
            }
        }
    }
    seen.into_iter().collect()
}

/// An owned child's live root; a Branch fork has no such ownership evidence.
pub(crate) fn owner(shared: &Shared, child: &str) -> Option<String> {
    shared.store.views().into_iter().find_map(|view| {
        let root = view.state["session"]["id"].as_str()?;
        (root != child
            && shared.channels.has_channel(root)
            && family(shared, root).iter().any(|id| id == child))
        .then(|| root.to_owned())
    })
}

/// A child view is read-only: Vibe owns its runtime, even after its turn settles or the app restarts.
pub(crate) async fn owned(shared: &Shared, id: &str) -> Result<bool, CoreError> {
    if shared.store.views().iter().any(|view| {
        view.session_id != id
            && family(shared, &view.session_id)
                .iter()
                .any(|child| child == id)
    }) {
        return Ok(true);
    }
    let state = match shared.store.view(id) {
        Some(view) => view.state,
        None => shared
            .gate
            .call("session/read", json!({"sessionId":id}), None)
            .await
            .map_err(|e| CoreError::from_gate("session/read", &e))?
            .result["state"]
            .clone(),
    };
    crate::session_ownership::owned_by_parent(shared, &state["session"]).await
}

/// Apply Stop generations and refuse controls before a child can be resumed or marked stuck.
pub(crate) async fn admit(
    shared: &Shared,
    method: &str,
    params: serde_json::Value,
    stop: Option<u64>,
) -> Result<serde_json::Value, CoreError> {
    let params = crate::stops::admit(shared, method, params, stop)?;
    let id = crate::routing::needs_channel(method, &params).or_else(|| {
        matches!(method, "runtime/read" | "session/resume")
            .then(|| params["sessionId"].as_str())
            .flatten()
    });
    if let Some(id) = id
        && !shared.channels.has_channel(id)
        && owned(shared, id).await?
    {
        return Err(CoreError::new(
            crate::ErrorCode::Denied,
            "Subagent chats are controlled by their parent chat.",
        )
        .with_method(method));
    }
    Ok(params)
}

/// Work on a child keeps the owning runtime alive even when the parent has finished its turn.
pub(crate) fn working(shared: &Shared, root: &str) -> bool {
    let ids = family(shared, root);
    let waiting = shared
        .gate
        .open_callbacks()
        .iter()
        .any(|card| ids.contains(&card.session_id))
        || shared.asks.list().iter().any(|ask| {
            ask.session
                .as_ref()
                .is_some_and(|id| id != root && ids.contains(id))
        });
    waiting
        || ids.iter().any(|id| {
            (id != root && crate::recovery::working(shared, id))
                || shared.store.view(id).is_some_and(|view| {
                    view.state["childSessions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|child| {
                            matches!(
                                child["status"]["type"].as_str(),
                                Some("running" | "blocked")
                            )
                        })
                })
        })
}

/// Cancels every wire callback immediately. `cancel_turn` is Vibe's supported targeted child interrupt:
/// harness `_session.py:2037-2059,2859-2871` reaches the child's active turn through the parent callback channel.
pub(crate) async fn cancel_cards(shared: &Shared, ids: &[String]) -> bool {
    let mut interrupted = false;
    for card in shared
        .gate
        .open_callbacks()
        .into_iter()
        .filter(|card| ids.contains(&card.session_id))
    {
        let output = if card.kind.as_str() == "approval" {
            json!({"type":"approval","decision":{"type":"cancel_turn"}})
        } else if card.kind.as_str() == "user_input" {
            json!({"type":"user_input","result":{"answers":[],"cancelled":true}})
        } else {
            continue;
        };
        if shared
            .gate
            .answer_callback(&card.session_id, &card.id, output)
            .await
            .is_ok()
        {
            interrupted |= card.kind.as_str() == "approval";
        }
    }
    interrupted
}

/// Stop the owned descendants while their root channel still exists. It must happen before closing the root.
pub(crate) async fn descendants(shared: &Shared, session: &str) -> Result<(), CoreError> {
    let ids = family(shared, session);
    if ids.len() > 1 && shared.channels.has_channel(session) {
        const METHOD: &str = "session/subagents/interrupt";
        let reply = shared
            .gate
            .call(
                METHOD,
                json!({"sessionId":session,"reason":"Stopped by the person"}),
                None,
            )
            .await
            .map_err(|e| CoreError::from_gate(METHOD, &e))?;
        if reply.result["results"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|r| r["outcome"] == "failed")
        {
            return Err(CoreError::bad_result(
                METHOD,
                "a child could not be interrupted",
            ));
        }
    }
    cancel_cards(shared, &ids).await;
    Ok(())
}

/// Interrupt a child without trying to resume a separately owned session.
pub(crate) async fn child(shared: &Shared, session: &str) -> Result<(), CoreError> {
    let ids = family(shared, session);
    if cancel_cards(shared, &ids).await {
        return Ok(());
    }
    // Its cancellation may already have closed the callback on the pump. Read the durable/live state to confirm.
    let reply = shared
        .gate
        .call("session/read", json!({"sessionId":session}), None)
        .await
        .map_err(|e| CoreError::from_gate("session/read", &e))?;
    let state = &reply.result["state"];
    if matches!(
        state["session"]["status"]["type"].as_str(),
        Some("running" | "blocked")
    ) {
        let parent = owner(shared, session).ok_or_else(|| CoreError::not_loaded(session))?;
        let turn = state["turns"]
            .as_array()
            .and_then(|t| t.last())
            .and_then(|t| t["id"].as_str())
            .ok_or_else(|| CoreError::bad_result("session/read", "turns"))?;
        let _routing = shared.channels.control_child(session, &parent);
        let result = shared
            .gate
            .call(
                "turn/interrupt",
                json!({"sessionId":session,"expectedTurnId":turn}),
                None,
            )
            .await;
        result.map_err(|e| CoreError::from_gate("turn/interrupt", &e))?;
    }
    Ok(())
}

/// The Stop command, serialized with app sends before closing the root channel.
pub(crate) async fn stop(
    core: &crate::Core,
    session: &str,
) -> Result<crate::CommandResult, CoreError> {
    stop_with(core, session, std::future::ready(Ok(()))).await?;
    Ok(crate::CommandResult::Reply {
        result: json!({"stopped":true}),
        client_user_message_id: None,
        user_entry_ids: Vec::new(),
    })
}

/// The same Stop path with a reserved child answer delivered after quiescing the root. A child completion must
/// not wake the parent's wait and trigger another provider request before Stop reaches it.
pub(crate) async fn stop_with<T>(
    core: &crate::Core,
    session: &str,
    answer: impl std::future::Future<Output = Result<T, CoreError>>,
) -> Result<T, CoreError> {
    let shared = &core.shared;
    if owner(shared, session).is_some() {
        child(shared, session).await?;
        answer.await
    } else {
        let canonical = shared.channels.canonical(session);
        let _held = crate::dispatch_compaction::stop_lock(shared, &canonical).await;
        let snapshot = crate::sessions::snapshot(shared, &canonical)?;
        // Quiesce the parent's turn before enumerating its children: it must not spawn a new one between the
        // subagent interrupt and channel close. Compaction keeps its existing close-without-waiting policy.
        if !crate::stops::compacting(shared, &canonical)
            && let Some(turn) = snapshot.state["session"]["status"]["activeTurnId"].as_str()
        {
            shared
                .gate
                .call(
                    "turn/interrupt",
                    json!({"sessionId":canonical,"expectedTurnId":turn}),
                    None,
                )
                .await
                .map_err(|e| CoreError::from_gate("turn/interrupt", &e))?;
        }
        let answered = answer.await;
        // A failed callback delivery still stops the tree; its reservation records the unconfirmed answer.
        descendants(shared, &canonical).await?;
        shared.channels.close(&canonical).await;
        answered
    }
}

/// Intercepts child interrupts before the ordinary path tries to attach the child separately.
pub(crate) async fn interrupt(
    shared: &Shared,
    method: &str,
    params: &serde_json::Value,
) -> Result<Option<vw_gate::Reply>, CoreError> {
    if method != "turn/interrupt" {
        return Ok(None);
    }
    let Some(id) = params["sessionId"].as_str() else {
        return Ok(None);
    };
    if let Some(parent) = owner(shared, id) {
        // Preserve the gate's full typed validation and Vibe's expected-turn check on raw calls too.
        let _routing = shared.channels.control_child(id, &parent);
        let reply = shared
            .gate
            .call(method, params.clone(), None)
            .await
            .map_err(|e| CoreError::from_gate(method, &e))?;
        cancel_cards(shared, &family(shared, id)).await;
        return Ok(Some(reply));
    }
    Ok(None)
}

/// A native card arriving after Stop is cancelled before any surface offers an Allow.
pub(crate) fn cancel_late(shared: &std::sync::Arc<Shared>, session: &str, owner: &str) -> bool {
    if !crate::stops::personally_stopped(shared, session)
        && !crate::stops::personally_stopped(shared, owner)
    {
        return false;
    }
    let shared = shared.clone();
    let session = session.to_owned();
    tokio::spawn(async move {
        cancel_cards(&shared, &[session]).await;
    });
    true
}
