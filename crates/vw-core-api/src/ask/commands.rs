//! Typed permission and Debug API operations. Ask records stay in memory.
//! Depends on the registry and typed command client; no route may answer a person-only ask.
use crate::{Command, CommandResult, Core, CoreError, ErrorCode, Origin};
use vw_ask::{Answer, AnsweredBy, Choice, Scope};

/// The session grants Vibe Work keeps for Vibe's approvals ("Allow for this session").
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AskGrantsRequest {
    /// Every kept grant, oldest first (each names its session).
    List {},
    /// Removes one grant: the next matching call in that session is asked again.
    Remove {
        /// The grant's id from `list`.
        id: String,
    },
    /// Removes several grants in one step ("Revoke all"). Ids already gone are skipped.
    RemoveMany {
        /// The grants' ids from `list` (at most 10 000).
        ids: Vec<String>,
    },
}
fn invalid(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidCommand, message)
}
impl Core {
    /// `askGrants`: list the session grants, remove one, or remove many in one step (the window and the Debug API
    /// act as the user; the agent has no route here).
    ///
    /// # Errors
    /// `invalidParams` when no grant has the id (`remove`), when none of the ids is still kept or the list is empty
    /// or longer than 10 000 (`removeMany`).
    pub(crate) fn ask_grants(&self, request: AskGrantsRequest) -> Result<CommandResult, CoreError> {
        let removed = match request {
            AskGrantsRequest::List {} => 0,
            AskGrantsRequest::Remove { id } => {
                if !self.shared.ask_grants.remove(&id) {
                    return Err(invalid("This permission is already gone."));
                }
                tracing::info!(grant = %id, "session grant removed");
                1
            }
            AskGrantsRequest::RemoveMany { ids } => {
                if ids.is_empty() || ids.len() > 10_000 {
                    return Err(invalid("Name between 1 and 10000 permissions to remove."));
                }
                let removed = self.shared.ask_grants.remove_many(&ids);
                if removed == 0 {
                    return Err(invalid("These permissions are already gone."));
                }
                tracing::info!(removed, "session grants removed");
                removed
            }
        };
        if removed > 0 {
            self.shared.emit(super::grants::event(&self.shared));
        }
        Ok(CommandResult::AskGrants {
            grants: self.shared.ask_grants.list(),
            offers: self.shared.ask_grants.offers(),
            removed: u32::try_from(removed).unwrap_or(u32::MAX),
        })
    }
    /// The session grants Vibe Work keeps, oldest first.
    #[must_use]
    pub fn ask_session_grants(&self) -> Vec<vw_ask::SessionGrant> {
        self.shared.ask_grants.list()
    }
    /// Pending permissions, containing metadata only.
    #[must_use]
    pub fn asks(&self) -> Vec<vw_ask::Ask> {
        self.shared.asks.list()
    }
    /// Newest retained permission outcomes, at most 500; never written to disk.
    #[must_use]
    pub fn ask_records(&self) -> Vec<vw_ask::AskRecord> {
        self.shared.asks.records()
    }
    /// Settle a pending permission as the Debug API's user.
    ///
    /// # Errors
    /// Refuses native person-only asks, unsupported scopes and late answers.
    pub async fn answer_ask(&self, id: &str, answer: Answer) -> Result<(), CoreError> {
        let ask = self
            .shared
            .asks
            .get(id)
            .ok_or_else(|| invalid("Already answered."))?;
        if ask.person_only {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "This ask is answered in Vibe Work's window only",
            ));
        }
        if !ask.offers.contains(&answer.answer) {
            return Err(invalid("This answer is not offered."));
        }
        if ask.kind.starts_with("chrome/ask/") {
            crate::chrome_control::check::answer(
                &self.shared,
                id,
                answer.answer,
                answer.note,
                AnsweredBy::DebugApi,
            )
            .await?;
        } else if let Some(callback) = id.strip_prefix("callback:") {
            let output = if ask.kind == "user_input" {
                if answer.answer != Choice::No {
                    return Err(invalid("Answer the question through the callback route."));
                }
                serde_json::json!({"type":"user_input","result":{"answers":[],"cancelled":true}})
            } else {
                serde_json::json!({"type":"approval","decision":{"type":match answer.answer { Choice::Once => "approve", Choice::Session => "approve_for_session", Choice::No => "deny" }}, "feedback":answer.note.map(|n| super::service::scrub_note(&n))})
            };
            self.execute(
                Origin::DebugApi,
                Command::AnswerCallback {
                    session_id: ask.session.unwrap_or_default(),
                    callback_id: callback.into(),
                    output,
                },
            )
            .await?;
        } else if let Some(proposal) = id.strip_prefix("proposal:") {
            self.execute(
                Origin::DebugApi,
                Command::DecideProtectedChange {
                    session_id: ask.session.unwrap_or_default(),
                    proposal_id: proposal.into(),
                    approve: answer.answer != Choice::No,
                    note: answer.note,
                },
            )
            .await?;
        } else {
            let outcome = match answer.answer {
                Choice::Once => super::allowed(AnsweredBy::DebugApi, Scope::Once),
                Choice::Session => super::allowed(AnsweredBy::DebugApi, Scope::Session),
                Choice::No => super::refused(AnsweredBy::DebugApi, answer.note),
            };
            if !self.shared.asks.settle(id, outcome) {
                return Err(invalid("Already answered."));
            }
        }
        Ok(())
    }
}
