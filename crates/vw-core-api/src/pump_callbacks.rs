//! Vibe's `callback/call` requests: registering each card with the gate, and the cards the core answers itself
//! (a session grant, a shipped skill read, read-aloud, on-demand tools, the user's guard answer).
use std::sync::{Arc, OnceLock, Weak};

use serde_json::{Value, json};
use tracing::warn;
use vw_protocol::ProtocolErrorCode;
use vw_rpc::{HandlerError, ServerRequest};

use crate::core::Shared;
use crate::event::CoreEvent;

/// The `callback/call` handler: registers the callback with the gate, tells subscribers, and acks
/// `{callbackId, accepted: true}` (the answer itself comes later as
/// `callback/result`). A callback the gate cannot register, or one that arrives on a channel other
/// than the one carrying its session, is refused with `invalid_params`. A card the user already
/// answered in a guard's dialog, or the sound-only read-aloud tool, is answered by the core
/// (`carry_answer`) instead of being shown.
pub(crate) fn callback_handler(
    slot: Arc<OnceLock<Weak<Shared>>>,
) -> impl Fn(ServerRequest) -> std::future::Ready<Result<Value, HandlerError>> + Send + Sync + 'static
{
    move |mut request: ServerRequest| {
        let Some(shared) = slot.get().and_then(Weak::upgrade) else {
            return std::future::ready(Err(HandlerError::new(
                ProtocolErrorCode::InternalError,
                "the client core is not ready",
            )));
        };
        crate::agent_tools::shell_commands::project(&shared, &mut request.params);
        #[cfg(feature = "video")]
        crate::video_projection::apply(&shared, &mut request.params);
        crate::edit_preview::project(&shared, &mut request.params);
        crate::chrome_control::approval::project(&shared, &mut request.params);
        if let Err(error) = crate::agent_tools::shell_commands::verified(&request.params) {
            return std::future::ready(Err(error));
        }
        let callback = request.params.get("callback");
        let text = |key: &str| {
            callback
                .and_then(|c| c.get(key))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let (callback_id, session_id) = (text("id"), text("sessionId"));
        let owner = shared.channels.key_of(&session_id).map(|key| key.1);
        if let (Some(owner), Some(arrived)) = (owner, request.channel_id)
            && owner != arrived
        {
            let total = shared.channels.note_foreign();
            warn!(
                arrived,
                owner, total, "callback for a session another channel carries; refused"
            );
            return std::future::ready(Err(HandlerError::new(
                ProtocolErrorCode::InvalidParams,
                "callback for a session of another channel",
            )));
        }
        // A subagent's card (its child session has no channel) is answered on the channel it came on, and
        // shown in the parent's view (the parent is the session that channel carries).
        let subagent = request.channel_id.and_then(|arrived| {
            shared
                .channels
                .note_callback(&session_id, &callback_id, arrived)
                .map(|parent_session_id| crate::event::SubagentCard {
                    parent_session_id,
                    entry: callback.cloned().unwrap_or(Value::Null),
                })
        });
        let outcome = match shared.gate.register_callback(&request.params) {
            Ok(kind) => {
                let entry = callback.cloned().unwrap_or(Value::Null);
                // A grant made on a subagent's card ends with the parent session that carries it.
                let owner = subagent
                    .as_ref()
                    .map_or_else(|| session_id.clone(), |s| s.parent_session_id.clone());
                if crate::stop_tree::cancel_late(&shared, &session_id, &owner)
                    || crate::ask::tool_calls::dispatch_transport(&shared, &entry, &owner)
                {
                    return std::future::ready(Ok(
                        json!({"callbackId":callback_id,"accepted":true}),
                    ));
                }
                let handle = tokio::runtime::Handle::try_current().ok();
                // Only with a runtime to answer on (the guard answer is consumed by the check).
                let carried = handle
                    .as_ref()
                    .and_then(|_| carried_for(&shared, &session_id, &entry, kind.as_str()));
                // Shipped reference reads are app policy, not a prompt: do not emit an AskOpened flash.
                if !matches!(carried, Some(Carried::OwnSkillRead)) {
                    crate::ask::callback_opened(&shared, &entry, kind.as_str(), &owner);
                }
                if let (Some(handle), Some(carried)) = (handle, carried) {
                    handle.spawn(carry_answer(
                        shared.clone(),
                        session_id,
                        callback_id.clone(),
                        carried,
                    ));
                } else {
                    // remembered so the user's answer counts for the guard inside our own tool.
                    if let Some(ledger) = crate::one_ask::of(&shared) {
                        ledger.card_opened(&entry);
                    }
                    shared.emit(CoreEvent::CallbackOpened {
                        session_id,
                        callback_id: callback_id.clone(),
                        kind: kind.as_str().to_owned(),
                        subagent,
                    });
                }
                Ok(json!({"callbackId": callback_id, "accepted": true}))
            }
            Err(fault) => {
                warn!(reason = %fault, "callback/call refused");
                Err(HandlerError::new(
                    ProtocolErrorCode::InvalidParams,
                    fault.to_string(),
                ))
            }
        };
        std::future::ready(outcome)
    }
}

/// Whether the core answers this card itself, checked in this order: a session grant ("Allow for this session"
/// earlier in this session, for this tool and exactly these permissions), the sound-only read-aloud tool, or the
/// user's guard answer for the same call (`one_ask`, which consumes it).
fn carried_for(
    shared: &Arc<Shared>,
    session_id: &str,
    entry: &Value,
    kind: &str,
) -> Option<Carried> {
    // A removed connector's card is shown instead (its tool's grants ended with it; this covers a card that arrived
    // between the pre-tool hook's check and the removal): the user's yes there becomes a refusal.
    let removed = entry["detail"]["effect"]["toolName"]
        .as_str()
        .is_some_and(|tool| shared.connectors.removed_tool(tool));
    if kind == "approval"
        && !removed
        && let Some(grant) = shared.ask_grants.matching(session_id, entry)
    {
        return Some(Carried::Grant(grant.id));
    }
    if crate::own_skill_reads::approve(&shared.market.builtin_roots, entry) {
        return Some(Carried::OwnSkillRead);
    }
    if shared.agent_tools.is_some() && crate::read_aloud::approve_without_dialog(entry) {
        return Some(Carried::ReadAloud);
    }
    if shared.agent_tools.is_some() && crate::on_demand::approve_without_dialog(entry) {
        return Some(Carried::OnDemand);
    }
    crate::one_ask::of(shared)
        .is_some_and(|l| l.answer_for_card(entry))
        .then_some(Carried::GuardAnswer)
}

/// Why the core answers an approval card itself instead of showing it.
#[derive(Clone)]
enum Carried {
    /// The user's yes in a guard's dialog for the same call (`one_ask`).
    GuardAnswer,
    /// The sound-only read-aloud tool (`read_aloud`).
    ReadAloud,
    /// Reading the app's own shipped Markdown reference.
    OwnSkillRead,
    /// A call that only turns tools on for the chat (`on_demand`): it does nothing itself.
    OnDemand,
    /// A session grant Vibe Work keeps ("Allow for this session" earlier, same tool and permissions); its id.
    Grant(String),
}

impl Carried {
    fn source(&self) -> &'static str {
        match self {
            Self::GuardAnswer => "user's guard answer",
            Self::ReadAloud => "read-aloud playback policy",
            Self::OwnSkillRead => "reading a shipped skill reference",
            Self::OnDemand => "turning tools on for the chat",
            Self::Grant(_) => "session grant",
        }
    }
    /// The ask record: a grant's answer is `Allowed { Session, Grant }` (the audit row "Allowed for this session
    /// (earlier answer)"); the other two answer this one call for the core.
    fn outcome(&self) -> vw_ask::Outcome {
        match self {
            Self::Grant(_) => {
                crate::ask::allowed(vw_ask::AnsweredBy::Grant, vw_ask::Scope::Session)
            }
            Self::GuardAnswer | Self::ReadAloud | Self::OwnSkillRead | Self::OnDemand => {
                crate::ask::allowed(vw_ask::AnsweredBy::Core, vw_ask::Scope::Once)
            }
        }
    }
}

/// Answers Vibe's approval card `callback_id` once (`approve`): the user's earlier guard answer, the read-aloud
/// playback policy, or a session grant. The audit row records the app origin and the ask record why. When the
/// send fails, the audit says the answer was not confirmed; a claimed callback cannot be offered again.
async fn carry_answer(
    shared: Arc<Shared>,
    session_id: String,
    callback_id: String,
    carried: Carried,
) {
    let source = carried.source();
    shared.ask_grants.forget_callback(&callback_id);
    let delivery = if matches!(carried, Carried::OwnSkillRead) {
        None
    } else {
        let Some(delivery) = shared.asks.reserve(
            &format!("callback:{callback_id}"),
            match &carried {
                Carried::Grant(_) => vw_ask::AnsweredBy::Grant,
                _ => vw_ask::AnsweredBy::Core,
            },
        ) else {
            return;
        };
        Some(delivery)
    };
    match shared
        .gate
        .answer_callback(&session_id, &callback_id, crate::one_ask::approve_once())
        .await
    {
        Ok(_) => {
            shared.emit(CoreEvent::CallbackAnswered {
                session_id: session_id.clone(),
                callback_id: callback_id.clone(),
                origin: crate::command::Origin::Ui,
            });
            if let Some(delivery) = delivery {
                delivery.confirmed(carried.outcome());
            }
            tracing::info!(
                session = %session_id,
                callback = %callback_id,
                source,
                "approval card answered once by the app"
            );
            // the grant's use count and last use (the Allowed page and the session's grant list show them).
            if let Carried::Grant(grant) = &carried
                && shared.ask_grants.record_use(grant, shared.asks.now_ms())
            {
                shared.emit(crate::ask::grants::event(&shared));
            }
        }
        Err(problem) => {
            warn!(reason = %problem, callback = %callback_id, "automatic approval answer not confirmed");
        }
    }
}

#[cfg(test)]
#[path = "ask/delivery_tests.rs"]
mod delivery_tests;
