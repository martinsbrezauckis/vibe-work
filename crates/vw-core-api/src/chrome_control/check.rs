//! `chrome_check`: the Chrome Control guard's question for one call, answered from the user's level.
//!
//! - **Nothing needed** (status, task, tab handover), the level includes the call's group, or an "Allow for this
//!   session" for that group on that site is kept: allow. Vibe's own approval card for the same call (when the
//!   session's access mode asks) is then answered for the user (`one_ask`): one approval.
//! - **Above the level**, bookmarks and history (at every level), closing one of the user's tabs while "Close tabs
//!   you opened" is off, or a tool Vibe Work does not know: the one `AskCard` opens in the session and this call
//!   waits in its hook until the person answers or stops the turn.
//!   "Allow for this session" keeps exactly that group on that site (a grant never reaches further than what
//!   the user saw); no card raises the level.
//!
//! Depends on: `vw_ask::chrome`, the ask service and grants, `one_ask`, `browser_delegation` (closing tabs).
//! Called by: `agent_tools::server` (hook credential only).
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use vw_ask::chrome::{self, Group, Need};
use vw_ask::{AnsweredBy, Choice, CoversPart, Outcome, Scope};

use super::{ChromeAskView, text};
use crate::core::Shared;
use crate::event::CoreEvent;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Question {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    parent_session_id: Option<String>,
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_input: Value,
}

fn allow() -> Value {
    json!({"decision": "allow"})
}

fn deny(reason: &str) -> Value {
    json!({"decision": "deny", "reason": reason})
}

/// The session a card for this call belongs to: the hook's session when the store has it, else its parent (a
/// subagent's call is shown in the parent's transcript).
fn owning_session(shared: &Shared, question: &Question) -> Option<String> {
    if shared.store.view(&question.session_id).is_some() {
        return Some(question.session_id.clone());
    }
    question
        .parent_session_id
        .clone()
        .filter(|p| shared.store.view(p).is_some())
}

fn turn_of(shared: &Shared, session: &str) -> Option<String> {
    let view = shared.store.view(session)?;
    view.state
        .pointer("/session/status/activeTurnId")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// What one ask is about: its kind, the card's reason and covers words, its group, and the session grant it would
/// keep (key and words for the grants list).
struct Asking {
    kind: &'static str,
    group: Option<Group>,
    reason: String,
    covers: Vec<CoversPart>,
    grant: (String, String),
}

/// The session grant key of `group` on `site` (`forms@shop.example`; just the group for a call naming no site).
fn grant_key(group: Group, site: Option<&str>) -> String {
    site.map_or_else(
        || group.id().to_owned(),
        |site| format!("{}@{site}", group.id()),
    )
}

/// What the user's level says about one call: run, or the card.
fn decide(
    shared: &Shared,
    need: Need,
    settings: vw_ask::ChromeSettings,
    session: &str,
    site: Option<&str>,
    restricted: bool,
) -> Option<Asking> {
    match need {
        Need::Nothing => None,
        Need::Group(group) => {
            if settings.allows(group) && !restricted {
                return None;
            }
            let key = grant_key(group, site);
            if let Some(grant) = shared
                .ask_grants
                .chrome_grant(session, &key)
                .filter(|_| !restricted)
            {
                if shared
                    .ask_grants
                    .record_use(&grant.id, shared.asks.now_ms())
                {
                    shared.emit(crate::ask::grants::event(shared));
                }
                return None;
            }
            Some(Asking {
                kind: group.ask_kind(),
                group: Some(group),
                reason: text::reason(settings, group),
                covers: text::covers(group, site),
                grant: (key, text::name(group.id()).to_owned()),
            })
        }
        Need::CloseUserTabs => {
            if crate::browser_delegation::load(shared).close_user_tabs
                || shared
                    .ask_grants
                    .chrome_grant(session, "closeTabs")
                    .is_some()
            {
                return None;
            }
            Some(Asking {
                kind: chrome::CLOSE_TABS_KIND,
                group: None,
                reason: text::close_tabs_reason(),
                covers: text::close_tabs_covers(),
                grant: ("closeTabs".to_owned(), "Close your tabs".to_owned()),
            })
        }
        Need::Unknown => Some(Asking {
            kind: chrome::OTHER_KIND,
            group: None,
            reason: text::UNKNOWN_REASON.to_owned(),
            covers: Vec::new(),
            grant: (String::new(), String::new()),
        }),
    }
}

/// The gateway shares the existing Chrome guard's long wait, before the MCP tool starts.
async fn session_gateway(shared: &Arc<Shared>, question: &Question) -> Value {
    let Some(session) = owning_session(shared, question) else {
        return deny("Not done: Vibe Work could not find this chat to ask about Chrome.");
    };
    if !crate::on_demand::present(shared, crate::on_demand::ToolSet::YourChrome) {
        // Let the gateway return its normal Set up explanation; there is no Chrome authority to enable yet.
        return allow();
    }
    let Some(tools) = shared.agent_tools.as_ref() else {
        return deny("Not done: Chrome Control is not available in this chat.");
    };
    match super::session::ensure(shared, &tools.on_demand, &session, &question.session_id).await {
        Ok(()) => allow(),
        Err(reason) => deny(&reason),
    }
}

/// Answers one `chrome_check`.
///
/// # Errors
/// A text reason when the request is not one.
pub(crate) async fn check(shared: &Arc<Shared>, args: Value) -> Result<Value, String> {
    if !shared.chrome_control.config.available() {
        return Ok(deny(super::setup::ABSENT));
    }
    let question: Question =
        serde_json::from_value(args).map_err(|_| "bad chrome_check request".to_owned())?;
    if vw_agent_tools::chrome_guard::is_session_gateway(&question.tool_name) {
        return Ok(session_gateway(shared, &question).await);
    }
    let leaf = question
        .tool_name
        .strip_prefix("mcp_vw.")
        .unwrap_or(&question.tool_name);
    if !super::gateway::is_tool(leaf) {
        return Err("unknown Chrome tool".to_owned());
    }
    let need = super::mapping::need(leaf, &question.tool_input);
    let action = super::mapping::action(leaf, &question.tool_input);
    let settings = shared.chrome_control.settings();
    let Some(session) = owning_session(shared, &question) else {
        return Ok(deny(
            "Not run: Vibe Work could not find this session to ask the user about this Chrome action.",
        ));
    };
    if !shared
        .agent_tools
        .as_ref()
        .is_some_and(|tools| tools.on_demand.chrome_allowed(&session))
    {
        return Ok(deny(
            "Not run: use_your_chrome must be allowed in this chat first.",
        ));
    }
    if super::gateway::restricted(shared, &action) == Some("never") {
        return Ok(deny(
            "Not run: this Chrome action was turned off in Chrome.",
        ));
    }
    let site = text::site(
        question
            .tool_input
            .get("args")
            .unwrap_or(&question.tool_input),
    );
    let asking = decide(
        shared,
        need,
        settings,
        &session,
        site.as_deref(),
        super::gateway::restricted(shared, &action) == Some("ask"),
    );
    let Some(asking) = asking else {
        return Ok(allow());
    };
    Ok(ask(shared, &question, &session, &action, &asking).await)
}

async fn ask(
    shared: &Arc<Shared>,
    question: &Question,
    session: &str,
    action: &str,
    asking: &Asking,
) -> Value {
    let summary = text::summary(
        &format!("chrome_{}", action.replace('.', "_")),
        question
            .tool_input
            .get("args")
            .unwrap_or(&question.tool_input),
    );
    let offers = if asking.covers.is_empty() {
        vec![Choice::Once, Choice::No]
    } else {
        vec![Choice::Once, Choice::Session, Choice::No]
    };
    let outcome = match super::pending::answer(shared, session, || {
        let handle = shared.asks.open_own(
            asking.kind,
            Some(question.session_id.clone()),
            vw_ask::Requester::Agent,
            offers,
            None,
        );
        let card = ChromeAskView {
            ask_id: handle.ask.id.clone(),
            session_id: session.to_owned(),
            turn_id: turn_of(shared, session),
            created_at_ms: shared.asks.now_ms(),
            tool: question.tool_name.clone(),
            summary: summary.clone(),
            group: asking.group,
            reason: asking.reason.clone(),
            covers: asking.covers.clone(),
            input: question.tool_input.clone(),
            open: true,
        };
        (handle, card)
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(reason) => return deny(&reason),
    };
    match outcome {
        Outcome::Allowed { scope, by } => {
            if scope == Scope::Session {
                let (key, label) = &asking.grant;
                let grant = shared.ask_grants.keep_chrome(
                    session,
                    key,
                    label,
                    asking.covers.clone(),
                    by,
                    shared.asks.now_ms(),
                );
                tracing::info!(session = %session, grant = %grant.id, "Chrome Control session grant kept");
                shared.emit(crate::ask::grants::event(shared));
            }
            allow()
        }
        other => deny(&vw_ask::agent_text(&other, "this Chrome action")),
    }
}

/// Publishes the card as closed when dropped.
pub(super) struct Closing {
    pub(super) shared: Arc<Shared>,
    pub(super) card: ChromeAskView,
}

impl Drop for Closing {
    fn drop(&mut self) {
        super::record_card_outcome(&self.shared, &self.card);
        let mut card = self.card.clone();
        card.open = false;
        publish(&self.shared, card);
    }
}

/// Keeps the card entry and tells the window (the session's transcript shows it at once).
pub(super) fn publish(shared: &Shared, view: ChromeAskView) {
    shared.chrome_control.upsert(view.clone());
    shared.emit(CoreEvent::ChromeAsk { entry: view });
}

/// The window's answer to a Chrome Control card (`chromeControl answer`). `by` is the person in the window or the
/// Debug API acting as the user.
///
/// # Errors
/// The card is closed already, or the answer is not offered.
pub(crate) fn answer(
    shared: &Arc<Shared>,
    ask_id: &str,
    choice: Choice,
    note: Option<String>,
    by: AnsweredBy,
) -> Result<(), crate::CoreError> {
    let invalid = |m: &str| crate::CoreError::new(crate::ErrorCode::InvalidCommand, m);
    let ask = shared
        .asks
        .get(ask_id)
        .filter(|a| a.kind.starts_with("chrome/ask/"))
        .ok_or_else(|| invalid("Already answered."))?;
    if !ask.offers.contains(&choice) {
        return Err(invalid("This answer is not offered."));
    }
    let outcome = match choice {
        Choice::Once => crate::ask::allowed(by, Scope::Once),
        Choice::Session => crate::ask::allowed(by, Scope::Session),
        Choice::No => crate::ask::refused(by, note),
    };
    let delivery = shared
        .asks
        .reserve(ask_id, by)
        .ok_or_else(|| invalid("Already answered."))?;
    // A No answers the held Chrome call with the declined result; the turn goes on.
    delivery.confirmed(outcome);
    Ok(())
}
