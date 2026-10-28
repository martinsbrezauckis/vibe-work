//! Vibe callback answers and settlement. The gate still validates and atomically claims every wire answer.
//! Depends on the shared registry and the session grants (`grants`); pump and command dispatch call these adapters.
//!
//! Vibe's approval choices are not forwarded as they are:
//! - "Allow for this session" (`approve_for_session` from the window or the Debug API) is sent to Vibe as
//!   `approve` and kept as a session grant here, so Vibe keeps no rule of its own;
//! - Vibe's "Always allow" (`approve_permanently`) is never offered and refused if a client sends it, because Vibe
//!   would write it into Vibe Work's `config.toml` for every project and later session;
//! - the gate refuses both decisions on the wire, so nothing else can send them either.
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::Arc;
use vw_ask::{AnsweredBy, Choice, Outcome, Scope};

use super::grants::Scopeable;

/// Opens the ask for a registered approval or question callback. `owner` is the session whose end ends a grant made on it (the card's
/// own session, or the parent session for a subagent's card).
pub(crate) fn callback_opened(shared: &Arc<Shared>, entry: &Value, kind: &str, owner: &str) {
    let _ = callback_opened_at(shared, entry, kind, owner, None);
}

/// [`callback_opened`] for an app-owned tool card: `generation` is the chat's Stop generation when the agent's call
/// was checked (`None`: taken here, before the ask opens). A Stop since then closes the card at once.
pub(crate) fn callback_opened_at(
    shared: &Arc<Shared>,
    entry: &Value,
    kind: &str,
    owner: &str,
    generation: Option<u64>,
) -> Option<super::service::Handle> {
    // Only the callbacks Vibe Work answers get an ask: a connector sign-in or a newer kind is Vibe's own to settle
    // (no answer is offered by Vibe Work).
    if !matches!(kind, "approval" | "user_input") {
        return None;
    }
    let id = entry["id"].as_str().unwrap_or_default();
    let session = entry["sessionId"].as_str().unwrap_or_default();
    let scope = if kind == "approval" {
        super::grants::scope_of(entry)
    } else {
        Scopeable::No
    };
    // Vibe's model offers every decision when `choices` is absent (`models.py:328`).
    let vibe_offers_session = entry["detail"]["choices"]
        .as_array()
        .is_none_or(|cs| cs.iter().any(|c| c == "approve_for_session"));
    let mut offers = vec![Choice::Once, Choice::No];
    // The session answer is offered only when a grant can actually be kept (no button that keeps nothing).
    let session_offered = vibe_offers_session && scope.keeps();
    if session_offered {
        offers.insert(1, Choice::Session);
    }
    let every_call = session_offered && scope.every_call();
    if kind == "approval" {
        shared.ask_grants.note_callback(
            id,
            session,
            owner,
            if session_offered {
                scope
            } else {
                Scopeable::No
            },
        );
        // The card's covers line comes from this offer: tell the clients before they draw the card.
        if session_offered {
            shared.emit(super::grants::event(shared));
        }
    }
    // The chat's Stop generation for this card: a Stop after it (a Stop before the answer also cancels the card)
    // cancels the held call (`stops.rs`).
    let generation = generation.unwrap_or_else(|| crate::stops::generation(shared, owner));
    let handle = shared.asks.open_callback(
        kind,
        Some(session.into()),
        offers,
        Some(format!("callback:{id}")),
        every_call,
    );
    if !super::tool_calls::local(id) || !handle.opened {
        return None;
    }
    crate::stops::opened_after_stop(shared, session, generation, &handle.ask.id);
    Some(handle)
}

/// The ask outcome a callback answer stands for.
fn outcome_of(output: &Value, by: AnsweredBy) -> Outcome {
    match output["decision"]["type"].as_str() {
        Some("cancel_turn") => Outcome::Cancelled {
            reason: "the turn was stopped".into(),
        },
        Some("deny" | "cancel") => {
            super::refused(by, output["feedback"].as_str().map(str::to_owned))
        }
        Some("approve_for_session") => super::allowed(by, Scope::Session),
        Some("approve_permanently") => super::allowed(by, Scope::Level),
        _ if output["result"]["cancelled"] == true => Outcome::Cancelled {
            reason: "the question was skipped".into(),
        },
        _ => super::allowed(by, Scope::Once),
    }
}

fn callback_answered(shared: &Shared, id: &str, output: &Value, by: AnsweredBy) -> bool {
    shared
        .asks
        .settle(&format!("callback:{id}"), outcome_of(output, by))
}

pub(crate) fn callback_closed(shared: &Shared, id: &str) {
    shared.ask_grants.forget_callback(id);
    if let Some(ask) = shared.asks.get(&format!("callback:{id}"))
        && let Some(view) = ask.session.as_deref().and_then(|s| shared.store.view(s))
        && let Some(entry) = view.state["history"]
            .as_array()
            .and_then(|h| h.iter().find(|e| e["callbackId"] == id))
        && entry["state"]["status"] == "answered"
    {
        callback_answered(shared, id, &entry["state"]["output"], AnsweredBy::Vibe);
        return;
    }

    shared.asks.settle(
        &format!("callback:{id}"),
        Outcome::Cancelled {
            reason: "Vibe closed the callback".into(),
        },
    );
}

fn refusal(message: &str) -> crate::CoreError {
    crate::CoreError::new(crate::ErrorCode::CallbackRefused, message)
}

/// The window's or the Debug API's approval answer as Vibe may receive it. "Allow for this session" becomes
/// `approve` (true = keep a session grant), but only where the ask offered it; "Always allow" is refused.
fn for_vibe(
    shared: &Shared,
    ask_id: &str,
    output: Value,
) -> Result<(Value, bool), crate::CoreError> {
    if output["type"] != "approval" {
        return Ok((output, false));
    }
    match output["decision"]["type"].as_str() {
        Some("approve_permanently") => Err(refusal(
            "Always allow is not offered: Vibe Work does not keep lasting Vibe rules. Allow once or for this session.",
        )),
        Some("approve_for_session") => {
            let offered = shared
                .asks
                .get(ask_id)
                .is_some_and(|ask| ask.offers.contains(&Choice::Session));
            if !offered {
                return Err(refusal("This answer is not offered."));
            }
            // The path scope only shaped Vibe's own grant; ours keeps the exact permissions Vibe sent (equality
            // match), so nothing of it is sent along with the once-only approve.
            Ok((
                json!({"type": "approval", "decision": {"type": "approve"}}),
                true,
            ))
        }
        _ => Ok((output, false)),
    }
}

/// A settled callback is a local refusal, before any ownership read or session reopening.
/// A fresh open ask takes precedence if Vibe reused the ID after a restart.
pub(crate) fn ensure_unanswered(
    shared: &Shared,
    callback_id: &str,
) -> Result<(), crate::CoreError> {
    let ask_id = format!("callback:{callback_id}");
    if shared.asks.get(&ask_id).is_none() && shared.asks.outcome(&ask_id).is_some() {
        return Err(crate::CoreError::new(
            crate::ErrorCode::CallbackRefused,
            "Already answered.",
        ));
    }
    Ok(())
}

/// Answer through the existing aliases with the same claim rules as /asks.
pub(crate) async fn answer(
    shared: &Arc<Shared>,
    origin: crate::Origin,
    session_id: String,
    callback_id: String,
    output: Value,
) -> Result<crate::CommandResult, crate::CoreError> {
    use crate::{CommandResult, CoreError, CoreEvent, ErrorCode};
    if super::tool_calls::local(&callback_id) {
        return super::tool_calls::answer(shared, origin, &session_id, &callback_id, &output).await;
    }
    let ask_id = format!("callback:{callback_id}");
    ensure_unanswered(shared, &callback_id)?;
    // Attribute a card clicked while the Debug API drives the window to that client.
    let by = if crate::command_origin::debug_api_acts(shared, origin) {
        vw_ask::AnsweredBy::DebugApi
    } else {
        vw_ask::AnsweredBy::Person
    };
    let (output, wants_session) = for_vibe(shared, &ask_id, output)?;
    // No channel step: a callback lives only on its session's open channel (a closed or
    // crashed channel ends it), so an answer for a session without one is refused by the
    // gate's callback book as unknown.
    let answered = output.clone();
    let output =
        shared
            .connectors
            .callback_answer(&session_id, &callback_id, &shared.gate, output)?;
    // A yes for a removed connector's tool is turned into a No by the core, not by the user. Its record
    // says so (by Core) and Vibe receives the connector sentence as is, never "the user said No"; no grant is kept.
    let by_core = output != answered;
    let keep_grant = wants_session && !by_core && output["decision"]["type"] == "approve";
    let outcome = if keep_grant {
        super::allowed(by, Scope::Session)
    } else {
        outcome_of(
            &output,
            if by_core {
                vw_ask::AnsweredBy::Core
            } else {
                by
            },
        )
    };
    let Some(delivery) = shared.asks.reserve(&ask_id, by) else {
        return Err(CoreError::new(
            ErrorCode::CallbackRefused,
            "Already answered.",
        ));
    };
    let mut output = output;
    let person_no = output["decision"]["type"] == "deny" && !by_core;
    let stop_owner = person_no
        .then(|| crate::stop_tree::owner(shared, &session_id))
        .flatten();
    if person_no {
        output = json!({"type":"approval", "decision":{"type":"cancel_turn"}});
    }
    let answer = output.clone();
    // Taken before the send: Vibe may close the callback before it answers, and the pump then forgets it.
    let claimed = keep_grant
        .then(|| shared.ask_grants.claim(&callback_id))
        .flatten();
    // Vibe may dispatch the approved MCP tool before acknowledging callback/result.
    // The validated answer has reserved the card; publish its exact-call allowance before sending it.
    if let Some(ledger) = crate::one_ask::of(shared) {
        ledger.card_answered(&session_id, &callback_id, &answer, origin);
    }
    let request = async {
        shared
            .gate
            .answer_callback(&session_id, &callback_id, output)
            .await
            .map_err(|e| CoreError::from_gate("callback/result", &e))
    };
    let sent = if let Some(root) = stop_owner {
        super::stop_with_no(shared, &root, request).await
    } else if person_no {
        super::root_no(shared, &session_id, request).await
    } else {
        request.await
    };
    shared.ask_grants.forget_callback(&callback_id);
    let result = sent.inspect_err(|_| {
        if let Some(ledger) = crate::one_ask::of(shared) {
            ledger.card_answer_failed(&session_id, &callback_id);
        }
        shared.gate.callback_closed(&callback_id);
    })?;
    delivery.confirmed(outcome);
    // Vibe took the once-only approve: keep the session grant (nothing is kept when the send failed).
    if let Some(pending) = claimed
        && let Some(grant) = shared.ask_grants.keep(pending, by, shared.asks.now_ms())
    {
        tracing::info!(session = %grant.session, tool = %grant.tool, grant = %grant.id, "session grant kept");
        shared.emit(super::grants::event(shared));
    }
    shared.emit(CoreEvent::CallbackAnswered {
        session_id,
        callback_id,
        origin,
    });
    Ok(CommandResult::Reply {
        result,
        client_user_message_id: None,
        user_entry_ids: Vec::new(),
    })
}
