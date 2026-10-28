//! Live protected-folder proposals: the authenticated agent captures, only an owner command decides.
//! Depends on vw-proposals, private review storage and session snapshots; called by tool/command dispatch.
//! Captured bytes and decision senders live only in memory. Stop invalidates captures already in flight.
//! One tool call waits at most [`DECISION_WAIT`] for the decision: Vibe ends a session MCP tool call after its fixed
//! default time and the tools program after its own call budget, both longer. A review still open then answers
//! Waiting with its id, stays open until decided or stopped, and `protected_change_status` collects the
//! finished outcome.
use crate::core::Shared;
use crate::protected_changes_store::{Reviews, Saved};
use crate::{
    CoreError, CoreEvent, ErrorCode, ProtectedChangeProblem, ProtectedChangeStatus as State,
    ProtectedChangeView, ProtectedFileView,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, watch};
use vw_proposals::{Decision, Proposal, Record, Status};
struct Pending {
    proposal: Proposal,
    reply: oneshot::Sender<String>,
}
/// The finished answer of one review (`None` while it is open), for the calls waiting on it.
type Outcome = watch::Receiver<Option<Result<String, String>>>;
/// Default for [`crate::AgentToolsConfig::decision_wait`]: under the tools program's 50 s call budget and Vibe's 60 s
/// limit for a session MCP tool call (the session protocol cannot raise it).
pub(crate) const DECISION_WAIT: Duration = Duration::from_secs(25);
struct Inner {
    reviews: Reviews,
    pending: HashMap<String, Pending>,
    /// Open reviews by id, with their session: what `propose` and `protected_change_status` wait on.
    outcomes: HashMap<String, (String, Outcome)>,
    epochs: HashMap<String, u64>,
}
pub(crate) struct Hub {
    inner: Mutex<Inner>,
    wait: Duration,
}
impl Hub {
    pub(crate) fn of(config: &crate::CoreConfig) -> Self {
        let mut hub = Self::open(
            config
                .sessions_dir
                .as_ref()
                .map(|p| p.join("protected-changes"))
                .or_else(|| {
                    config
                        .agent_tools
                        .as_ref()
                        .map(|t| t.data_dir.join("protected-changes"))
                }),
        );
        if let Some(tools) = &config.agent_tools {
            hub.wait = tools.decision_wait.min(DECISION_WAIT);
        }
        hub
    }
    pub(crate) fn open(directory: Option<PathBuf>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                reviews: Reviews::open(directory),
                pending: HashMap::new(),
                outcomes: HashMap::new(),
                epochs: HashMap::new(),
            }),
            wait: DECISION_WAIT,
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
    pub(crate) fn shutdown(&self) {
        self.lock().pending.clear();
    }
    pub(crate) fn overlay(&self, session_id: &str, state: &mut Value) {
        self.lock().reviews.overlay(session_id, state);
    }
}
fn invalid(reason: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidCommand, reason)
}
fn emit(shared: &Shared, view: ProtectedChangeView) {
    shared.emit(CoreEvent::ProtectedChange { proposal: view });
}

/// The two agent tools of this module: `propose_protected_change` and `protected_change_status`.
pub(crate) async fn tool(
    shared: &Arc<Shared>,
    session_id: &str,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    if tool == vw_agent_tools::wire::tools::PROTECTED_CHANGE_STATUS {
        status(shared, session_id, &args).await
    } else {
        propose(shared, session_id, args).await
    }
}
async fn propose(shared: &Arc<Shared>, session_id: &str, args: Value) -> Result<Value, String> {
    let started = Instant::now();
    // Not paused by the vault's agent access switch (it covers vault data only): the proposal is itself the card
    // that asks the user.
    if !shared.channels.has_channel(session_id) || shared.store.view(session_id).is_none() {
        return Err("this session is not live in Vibe Work right now".into());
    }
    let epoch = *shared
        .protected_changes
        .lock()
        .epochs
        .get(session_id)
        .unwrap_or(&0);
    let root = shared.instructions.own_home.clone();
    let sid = session_id.to_owned();
    let proposal = tokio::task::spawn_blocking(move || {
        crate::protected_changes_validate::capture(&root, &sid, args)
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut view = review(&proposal);
    view.turn_id = shared.store.view(session_id).and_then(|session| {
        session
            .state
            .pointer("/session/status/activeTurnId")
            .and_then(Value::as_str)
            .or_else(|| {
                session.state["turns"]
                    .as_array()?
                    .iter()
                    .rev()
                    .find(|turn| turn["status"] == "in_progress")?["id"]
                    .as_str()
            })
            .map(str::to_owned)
    });
    let id = view.id.clone();
    let (reply, received) = oneshot::channel();
    let mut ask = shared.asks.open(
        "protected_change",
        Some(session_id.into()),
        vw_ask::Requester::Agent,
        vec![vw_ask::Choice::Once, vw_ask::Choice::No],
        Some(format!("proposal:{id}")),
    );
    ask.cancel_on_drop = true;
    let (done, outcome) = watch::channel(None);
    {
        let mut hub = shared.protected_changes.lock();
        if *hub.epochs.get(session_id).unwrap_or(&0) != epoch {
            return Err("Declined: the turn was stopped.".into());
        }
        if hub
            .pending
            .values()
            .filter(|p| p.proposal.session_id() == session_id)
            .count()
            >= 8
        {
            return Err("at most 8 pending proposals per session".into());
        }
        hub.reviews.save(Saved {
            view: view.clone(),
            record: None,
        })?;
        hub.pending
            .insert(view.id.clone(), Pending { proposal, reply });
        hub.outcomes
            .insert(id.clone(), (session_id.to_owned(), outcome.clone()));
        emit(shared, view);
    }
    // The review outlives this call: its owner holds the ask until a decision or a stop, then
    // publishes the agent's answer and forgets the open entry (a later status call reads the saved review).
    let owner = shared.clone();
    let owner_id = id.clone();
    tokio::spawn(async move {
        let answer = decision(ask, received).await;
        owner.protected_changes.lock().outcomes.remove(&owner_id);
        let _ = done.send(Some(answer));
    });
    let wait = shared
        .protected_changes
        .wait
        .saturating_sub(started.elapsed());
    wait_for(shared, &id, outcome, wait).await
}
/// Waits for the review's own outcome: a decision, a stop or a cancellation.
async fn decision(
    mut ask: crate::ask::service::Handle,
    mut received: oneshot::Receiver<String>,
) -> Result<String, String> {
    tokio::select! {
        answer = &mut received => Ok(match answer {
            Ok(answer) => answer,
            Err(_) => vw_ask::agent_text(&ask.closed().await, &ask.ask.title),
        }),
        outcome = ask.closed() => {
            if matches!(outcome, vw_ask::Outcome::Cancelled { .. }) {
                Ok(vw_ask::agent_text(&outcome, &ask.ask.title))
            } else { received.await.map_err(|_| "the proposal is no longer pending".to_owned()) }
        }
    }
}
/// One tool call's wait of at most `wait` on review `id`: the finished answer, or a Waiting answer naming the id.
async fn wait_for(
    shared: &Arc<Shared>,
    id: &str,
    mut outcome: Outcome,
    wait: Duration,
) -> Result<Value, String> {
    // A dropped socket cancels the review it was waiting on; no orphaned actionable card survives a lost caller.
    // Between calls (after a Waiting answer) nobody waits and the review simply stays open.
    let mut cancellation = Cancel {
        shared: Arc::downgrade(shared),
        id: Some(id.to_owned()),
    };
    let finished = tokio::time::timeout(wait, outcome.wait_for(Option::is_some)).await;
    cancellation.id = None;
    match finished {
        Ok(Ok(answer)) => answer
            .clone()
            .unwrap_or_else(|| Err("the proposal is no longer pending".into()))
            .map(Value::String),
        Ok(Err(_)) => Err("the proposal is no longer pending".into()),
        Err(_) => {
            // Open ask: the user has not decided. No open ask: the decision arrived and is being carried out.
            let text = shared.asks.get(&format!("proposal:{id}")).map_or_else(
                || {
                    "Waiting: the user's decision on these protected-folder changes is being carried out. Do not \
                     propose the same changes again."
                        .to_owned()
                },
                |_| "Waiting: the user has not decided on these protected-folder changes yet. Nothing has changed so far.".to_owned(),
            );
            Ok(serde_json::json!({
                "status": "waiting",
                "proposalId": id,
                "text": text,
                "next": "call protected_change_status with this proposalId to get the outcome",
            }))
        }
    }
}
/// `protected_change_status`: the outcome of a review this session proposed, waiting like `propose` while it is open.
async fn status(shared: &Arc<Shared>, session_id: &str, args: &Value) -> Result<Value, String> {
    let started = Instant::now();
    let id = args["proposalId"]
        .as_str()
        .ok_or("protected_change_status needs a proposalId")?;
    let unknown = || "no such proposal in this session".to_owned();
    let outcome = {
        let hub = shared.protected_changes.lock();
        match hub.outcomes.get(id) {
            Some((session, outcome)) if session == session_id => outcome.clone(),
            Some(_) => return Err(unknown()),
            None => {
                let saved = hub
                    .reviews
                    .entries
                    .get(id)
                    .filter(|saved| saved.view.session_id == session_id)
                    .ok_or_else(unknown)?;
                if saved.view.status == State::Pending {
                    return Err("the proposal is no longer pending".into());
                }
                return Ok(Value::String(reply(&saved.view)));
            }
        }
    };
    let wait = shared
        .protected_changes
        .wait
        .saturating_sub(started.elapsed());
    wait_for(shared, id, outcome, wait).await
}
struct Cancel {
    shared: std::sync::Weak<Shared>,
    id: Option<String>,
}
impl Drop for Cancel {
    fn drop(&mut self) {
        if let (Some(shared), Some(id)) = (self.shared.upgrade(), self.id.take()) {
            let _ = settle(
                &shared,
                &id,
                false,
                None,
                Some(vw_ask::Outcome::Cancelled {
                    reason: "the caller disconnected".into(),
                }),
            );
        }
    }
}
fn review(p: &Proposal) -> ProtectedChangeView {
    ProtectedChangeView {
        id: p.id().into(),
        session_id: p.session_id().into(),
        turn_id: None,
        reason: p.reason().into(),
        created_at_ms: p.created_at_ms(),
        decided_at_ms: None,
        status: State::Pending,
        files: p
            .review()
            .into_iter()
            .map(|f| ProtectedFileView {
                path: f.path.to_string_lossy().into_owned(),
                kind: match f.kind {
                    vw_proposals::Kind::Add => "add",
                    vw_proposals::Kind::Edit => "edit",
                    vw_proposals::Kind::Remove => "remove",
                }
                .into(),
                content_kind: if f.content_kind == vw_proposals::ContentKind::Text {
                    "text"
                } else {
                    "binary"
                }
                .into(),
                before_bytes: f.before_bytes,
                after_bytes: f.after_bytes,
                added_lines: f.added_lines,
                removed_lines: f.removed_lines,
                diff: f.diff,
                diff_truncated: false,
            })
            .collect(),
        applied: Vec::new(),
        problem: None,
        note: None,
        stopped: false,
    }
}

/// Handles only the typed window/Debug API command; the MCP credential table contains no decision tool.
pub(crate) async fn decide(
    shared: &Arc<Shared>,
    session_id: String,
    id: String,
    approve: bool,
    note: Option<String>,
    by: vw_ask::AnsweredBy,
) -> Result<ProtectedChangeView, CoreError> {
    if !shared.channels.has_channel(&session_id) {
        return Err(invalid("this proposal has no live session"));
    }
    if note.as_ref().is_some_and(|s| {
        s.len() > 8000
            || s.chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    }) {
        return Err(invalid(
            "the decline note must be plain text of at most 8000 bytes",
        ));
    }
    {
        let hub = shared.protected_changes.lock();
        let pending = hub
            .pending
            .get(&id)
            .ok_or_else(|| invalid("this proposal is no longer pending"))?;
        if session_id != pending.proposal.session_id() {
            return Err(invalid("this proposal belongs to another session"));
        }
    }
    if !approve && crate::stop_tree::owner(shared, &session_id).is_none() {
        crate::ask::stop_after_no(shared, &session_id).await?;
        return shared
            .protected_changes
            .lock()
            .reviews
            .entries
            .get(&id)
            .map(|saved| saved.view.clone())
            .ok_or_else(|| invalid("this review is unavailable"));
    }
    let shared = shared.clone();
    tokio::task::spawn_blocking(move || {
        settle(
            &shared,
            &id,
            approve,
            note.clone(),
            Some(if approve {
                crate::ask::allowed(by, vw_ask::Scope::Once)
            } else {
                crate::ask::refused(by, note)
            }),
        )
    })
    .await
    .map_err(|e| CoreError::new(ErrorCode::Internal, e.to_string()))?
}
#[allow(
    clippy::too_many_lines,
    reason = "ownership, decision and filesystem application stay ordered"
)]
fn settle(
    shared: &Shared,
    id: &str,
    approve: bool,
    note: Option<String>,
    outcome: Option<vw_ask::Outcome>,
) -> Result<ProtectedChangeView, CoreError> {
    let stopped = matches!(&outcome, Some(vw_ask::Outcome::Cancelled { reason }) if reason == "the turn was stopped");
    let (pending, mut view) = {
        let mut hub = shared.protected_changes.lock();
        let ask_id = format!("proposal:{id}");
        let settled = shared.asks.settle(
            &ask_id,
            outcome.unwrap_or_else(|| {
                if approve {
                    crate::ask::allowed(vw_ask::AnsweredBy::Person, vw_ask::Scope::Once)
                } else {
                    crate::ask::refused(vw_ask::AnsweredBy::Person, note.clone())
                }
            }),
        );
        if approve && !settled {
            return Err(invalid("Already answered."));
        }

        let mut view = hub
            .reviews
            .entries
            .get(id)
            .ok_or_else(|| invalid("this review is unavailable"))?
            .view
            .clone();
        let pending = hub
            .pending
            .remove(id)
            .ok_or_else(|| invalid("this proposal is no longer pending"))?;
        if approve {
            view.status = State::Applying;
            if let Some(saved) = hub.reviews.entries.get_mut(id) {
                saved.view = view.clone();
            }
            emit(shared, view.clone());
        }
        (pending, view)
    };
    let record = pending
        .proposal
        .apply(if approve {
            Decision::Approve
        } else {
            Decision::Decline
        })
        .unwrap_or_else(|error| *error.record);
    finish(
        &mut view,
        &record,
        note.map(|n| crate::ask::service::scrub_note(&n)),
        stopped,
    );
    {
        let mut hub = shared.protected_changes.lock();
        let saved = Saved {
            view: view.clone(),
            record: Some(record),
        };
        if let Err(message) = hub.reviews.save(saved.clone()) {
            view.status = State::Failed;
            view.problem = Some(ProtectedChangeProblem {
                path: None,
                message: format!("The decision record could not be saved: {message}"),
            });
            hub.reviews.entries.insert(
                id.into(),
                Saved {
                    view: view.clone(),
                    ..saved
                },
            );
        }
        emit(shared, view.clone());
    }
    let _ = pending.reply.send(reply(&view));
    Ok(view)
}
fn finish(view: &mut ProtectedChangeView, record: &Record, note: Option<String>, stopped: bool) {
    view.status = match record.decision {
        Status::Approved => State::Applied,
        Status::Declined => State::Declined,
        Status::Stale => State::Stale,
        Status::Failed => State::Failed,
    };
    view.decided_at_ms = Some(record.decided_at_ms);
    view.applied = record
        .applied
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    view.problem = record.problem.as_ref().map(|p| ProtectedChangeProblem {
        path: p.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        message: p.message.clone(),
    });
    view.note = note.filter(|s| !s.trim().is_empty());
    view.stopped = stopped;
}
/// The agent's answer for a finished review (also what a later status call reads from the saved review).
fn reply(view: &ProtectedChangeView) -> String {
    match view.status {
        State::Applied => format!("applied: {}", view.applied.join(", ")),
        State::Declined if view.stopped => vw_ask::agent_text(
            &vw_ask::Outcome::Cancelled {
                reason: "the turn was stopped".into(),
            },
            "these protected-folder changes",
        ),
        State::Declined => vw_ask::agent_text(
            &crate::ask::refused(vw_ask::AnsweredBy::Person, view.note.clone()),
            "these protected-folder changes",
        ),
        State::Stale => format!(
            "Nothing was changed: {} changed after this was proposed. Ask Vibe to propose it again.",
            view.problem
                .as_ref()
                .and_then(|p| p.path.as_deref())
                .unwrap_or("a file")
        ),
        _ => format!(
            "Stopped at {}: {}. Applied before that: {}. Not applied: {}.",
            view.problem
                .as_ref()
                .and_then(|p| p.path.as_deref())
                .unwrap_or("the proposal"),
            view.problem
                .as_ref()
                .map_or("application failed", |p| p.message.as_str()),
            view.applied.join(", "),
            view.files
                .iter()
                .filter(|f| !view.applied.contains(&f.path))
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
/// Invalidates capture races, then declines every unanswered review in this session.
pub(crate) fn stop(shared: &Shared, session_id: &str) {
    let ids = {
        let mut hub = shared.protected_changes.lock();
        *hub.epochs.entry(session_id.into()).or_default() += 1;
        hub.pending
            .values()
            .filter(|p| p.proposal.session_id() == session_id)
            .map(|p| p.proposal.id().to_owned())
            .collect::<Vec<_>>()
    };
    for id in ids {
        let _ = settle(
            shared,
            &id,
            false,
            None,
            Some(vw_ask::Outcome::Cancelled {
                reason: "the turn was stopped".into(),
            }),
        );
    }
}
