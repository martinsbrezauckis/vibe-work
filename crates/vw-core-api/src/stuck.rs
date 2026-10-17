//! Stuck sessions: sessions whose reopen after a server restart failed or did not finish in time.
//!
//! Users must never get stuck mid-work, and the core makes no automatic retries.
//! So after the first failed reopen the session is marked stuck; its commands are refused at once
//! (`sessionStuck`, nothing waits); its history stays readable through the lobby; and the user has two
//! explicit actions: try again (one bounded attempt, cancellable) or continue in a new session (same
//! folder, same profile, linked).
//!
//! Depends on: `core` (`Core`, `Shared`), `recovery` (`open_session`), `restore` (profile of the new
//! session), `access`, `dispatch` (`session_call`), `event`, `view`. Called by: `recovery::reopen` (marks),
//! `dispatch` (refusals, the three commands), `core::status`.
//! Invariants:
//! - A stuck session is never opened implicitly: only `retryReopen` sends a resume for it, exactly one per
//!   call, bounded by `CoreConfig::retry_reopen_timeout`.
//! - Cancelling drops the attempt: the request future, its channel (closed on the server when dropped) and
//!   the per-session locks go with it.
//! - Other sessions never wait for a stuck one (no lock is shared).
//! - A retry's completion clears only its own cancellation switch; a replacement keeps its owner.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::access::FALLBACK_PROFILE;
use crate::command::{CommandResult, Origin};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::view::StuckView;

/// The text users see for a stuck session.
pub(crate) const STUCK_TEXT: &str = "Vibe can't reopen this session right now";

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The stuck sessions and the running retries.
#[derive(Default)]
pub(crate) struct StuckSessions {
    /// Stuck session -> the session `continueInNewSession` started for it.
    stuck: Mutex<BTreeMap<String, Option<String>>>,
    /// Running `retryReopen` attempts, each with its cancel switch.
    retries: Mutex<HashMap<String, Retry>>,
}

/// The sender and the unique owner whose completion may remove it.
struct Retry {
    owner: Arc<()>,
    cancel: oneshot::Sender<()>,
}

impl StuckSessions {
    /// Marks `id` stuck. Returns false when it already was.
    pub(crate) fn mark(&self, id: &str) -> bool {
        let mut stuck = locked(&self.stuck);
        if stuck.contains_key(id) {
            return false;
        }
        stuck.insert(id.to_owned(), None);
        true
    }

    /// True while `id` is stuck.
    pub(crate) fn is_stuck(&self, id: &str) -> bool {
        locked(&self.stuck).contains_key(id)
    }

    /// Unmarks `id`. Returns false when it was not stuck.
    fn clear(&self, id: &str) -> bool {
        locked(&self.stuck).remove(id).is_some()
    }

    fn set_continued(&self, id: &str, new_id: &str) {
        if let Some(slot) = locked(&self.stuck).get_mut(id) {
            *slot = Some(new_id.to_owned());
        }
    }

    fn continued_in(&self, id: &str) -> Option<String> {
        locked(&self.stuck).get(id).cloned().flatten()
    }

    /// The stuck sessions for the status report.
    pub(crate) fn list(&self) -> Vec<StuckView> {
        locked(&self.stuck)
            .iter()
            .map(|(id, link)| StuckView {
                session_id: id.clone(),
                continued_in: link.clone(),
            })
            .collect()
    }

    /// Registers a retry of `id`; a retry already running is cancelled (only one at a time).
    fn begin_retry(&self, id: &str) -> (Arc<()>, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        let owner = Arc::new(());
        let retry = Retry {
            owner: owner.clone(),
            cancel: tx,
        };
        if let Some(old) = locked(&self.retries).insert(id.to_owned(), retry) {
            let _ = old.cancel.send(());
        }
        (owner, rx)
    }

    fn end_retry(&self, id: &str, owner: &Arc<()>) {
        let mut retries = locked(&self.retries);
        if retries
            .get(id)
            .is_some_and(|retry| Arc::ptr_eq(&retry.owner, owner))
        {
            retries.remove(id);
        }
    }

    /// Cancels the running retry of `id`. Returns false when none runs.
    fn cancel(&self, id: &str) -> bool {
        locked(&self.retries)
            .remove(id)
            .is_some_and(|retry| retry.cancel.send(()).is_ok())
    }
}

/// The refusal for a command of a stuck session.
pub(crate) fn refusal(id: &str, method: &str) -> CoreError {
    CoreError::new(
        ErrorCode::SessionStuck,
        format!("{STUCK_TEXT} (session {id}): try again or continue in a new session"),
    )
    .with_method(method)
}

impl Core {
    /// Runs an implicit open of `id` (a resume before a call, or a client's resume) bounded by
    /// `CoreConfig::reopen_timeout`. When it times out, or Vibe answers it with an error other than
    /// `not_found`, the session is marked stuck and the call fails with
    /// `sessionStuck`: nobody waits longer, and the user gets the two explicit actions (live: after a
    /// quit too, a session that was waiting on a bash approval may never answer a resume again).
    pub(crate) async fn bounded_open(
        &self,
        id: &str,
        open: impl std::future::Future<Output = Result<(), CoreError>>,
    ) -> Result<(), CoreError> {
        let bound = self.shared.config.reopen_timeout;
        let outcome = match tokio::time::timeout(bound, open).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => error,
            Err(_) => CoreError::new(
                ErrorCode::Timeout,
                format!("no answer to the reopen within {}s", bound.as_secs()),
            ),
        };
        // Not `serverUnavailable`: a restart in progress is no reason to give up on a session.
        let stuck = matches!(outcome.code, ErrorCode::Timeout | ErrorCode::ServerError)
            && outcome.server_code.as_deref() != Some("not_found");
        if !stuck {
            return Err(outcome);
        }
        self.note_stuck(id);
        Err(refusal(id, "session/resume"))
    }

    /// Marks a session stuck after a failed reopen and tells subscribers.
    pub(crate) fn note_stuck(&self, id: &str) {
        if self.shared.stuck.mark(id) {
            warn!(
                session_id = id,
                "session is stuck: Vibe could not reopen it"
            );
            self.emit_stuck(id, true);
        }
    }

    fn emit_stuck(&self, id: &str, stuck: bool) {
        self.shared.emit(CoreEvent::SessionStuck {
            session_id: id.to_owned(),
            stuck,
            continued_in: self.shared.stuck.continued_in(id),
        });
    }

    /// `retryReopen`: one bounded attempt to reopen a stuck (or channel-less) session.
    pub(crate) async fn retry_reopen(
        &self,
        origin: Origin,
        id: &str,
    ) -> Result<CommandResult, CoreError> {
        let (owner, mut cancel) = self.shared.stuck.begin_retry(id);
        let bound = self.shared.config.retry_reopen_timeout;
        let attempt = tokio::time::timeout(bound, self.open_session(origin, id));
        let outcome = tokio::select! {
            done = attempt => Some(done),
            _ = &mut cancel => None,
        };
        self.shared.stuck.end_retry(id, &owner);
        match outcome {
            Some(Ok(Ok(()))) => {
                if self.shared.stuck.clear(id) {
                    info!(session_id = id, "a stuck session was reopened");
                    self.emit_stuck(id, false);
                }
                Ok(reply(json!({"reopened": true})))
            }
            Some(Ok(Err(error))) => Err(error),
            Some(Err(_)) => Err(CoreError::new(
                ErrorCode::SessionStuck,
                format!("{STUCK_TEXT}: no answer within {}s", bound.as_secs()),
            )
            .with_method("session/resume")),
            None => Err(CoreError::new(
                ErrorCode::Cancelled,
                "stopped waiting for Vibe to reopen the session",
            )
            .with_method("session/resume")),
        }
    }

    /// `cancelReopen`.
    pub(crate) fn cancel_reopen(&self, id: &str) -> CommandResult {
        reply(json!({"cancelled": self.shared.stuck.cancel(id)}))
    }

    /// `continueInNewSession`: `session/start` in the stuck session's folder with its profile (the
    /// remembered one, else Vibe's record, else `ask`; a profile the core may not select itself becomes
    /// `ask`), then the link.
    pub(crate) async fn continue_in_new(
        &self,
        origin: Origin,
        id: &str,
    ) -> Result<CommandResult, CoreError> {
        let state = match self.shared.store.view(id) {
            Some(view) => view.state,
            None => self
                .shared
                .gate
                .call("session/read", json!({"sessionId": id}), None)
                .await
                .map_err(|e| CoreError::from_gate("session/read", &e))?
                .result
                .get("state")
                .cloned()
                .unwrap_or(Value::Null),
        };
        let mut agent = self
            .shared
            .access
            .remembered(id)
            .or_else(|| {
                state
                    .pointer("/session/agent/name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| FALLBACK_PROFILE.to_owned());
        let mut config = json!({"agent": agent});
        if let Some(cwd) = state.pointer("/session/cwd").and_then(Value::as_str) {
            config["cwd"] = json!(cwd);
        }
        let params = json!({"agentConfig": config});
        if self
            .shared
            .gate
            .check("session/start", &params, None)
            .is_err()
        {
            FALLBACK_PROFILE.clone_into(&mut agent);
            config["agent"] = json!(agent);
        }
        // The continued session stays in the stuck session's project.
        let hint = crate::projects_filing::inherit_hint(&self.shared, id);
        let result = Box::pin(crate::projects_filing::with_hint(
            hint,
            self.session_call(origin, "session/start", json!({"agentConfig": config})),
        ))
        .await?;
        if let CommandResult::Session { snapshot, .. } = &result {
            self.shared.stuck.set_continued(id, &snapshot.session_id);
            info!(
                session_id = id,
                new = snapshot.session_id,
                agent,
                "continued a stuck session in a new one"
            );
            if self.shared.stuck.is_stuck(id) {
                self.emit_stuck(id, true);
            }
        }
        Ok(result)
    }
}

fn reply(result: Value) -> CommandResult {
    CommandResult::Reply {
        result,
        client_user_message_id: None,
        user_entry_ids: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::StuckSessions;
    use tokio::sync::oneshot::error::TryRecvError;

    #[test]
    fn overlapping_retry_cleanup_keeps_the_newest_cancellable() {
        let stuck = StuckSessions::default();
        let (first_owner, mut first) = stuck.begin_retry("session");
        let (second_owner, mut second) = stuck.begin_retry("session");
        assert_eq!(
            first.try_recv(),
            Ok(()),
            "replacement cancels the older attempt"
        );
        stuck.end_retry("session", &first_owner);
        assert_eq!(
            second.try_recv(),
            Err(TryRecvError::Empty),
            "older cleanup must keep the replacement active"
        );
        assert!(
            stuck.cancel("session"),
            "the replacement remains separately cancellable"
        );
        assert_eq!(second.try_recv(), Ok(()));
        stuck.end_retry("session", &second_owner);
    }

    #[test]
    fn delayed_cleanup_after_cancel_keeps_a_later_retry_and_other_sessions() {
        let stuck = StuckSessions::default();
        let (old_owner, mut old) = stuck.begin_retry("session");
        assert!(stuck.cancel("session"));
        assert_eq!(old.try_recv(), Ok(()));
        let (new_owner, mut new) = stuck.begin_retry("session");
        let (other_owner, mut other) = stuck.begin_retry("other");
        stuck.end_retry("session", &old_owner);
        assert_eq!(new.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(other.try_recv(), Err(TryRecvError::Empty));
        stuck.end_retry("session", &new_owner);
        assert!(
            !stuck.cancel("session"),
            "own completion removed its sender"
        );
        assert!(stuck.cancel("other"));
        assert_eq!(other.try_recv(), Ok(()));
        stuck.end_retry("other", &other_owner);
    }
}
