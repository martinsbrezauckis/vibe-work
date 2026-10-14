//! The book of open server callbacks: which ids exist, of which kind, for which session, and
//! whether they were already answered.
//!
//! Depends on: `callback_shapes`, error. Called by: gate (`register_callback`, `answer_callback`,
//! `callback_closed`, `session_closed`).
//! Invariants:
//! - An answer is accepted only for an open callback of the same session whose kind has a verified
//!   output shape, and only if the output has exactly that shape.
//! - `claim` validates and marks the callback answered in one step under the caller's lock, before
//!   anything is awaited, so two racing answers can never both be sent.
//! - A refused answer leaves the callback open. A claimed callback stays claimed whatever the
//!   outcome of the send: the gate never sends a second answer for it.
//! - `connector_auth` and unknown kinds are listed (the UI must see them) but never answered here.

use std::collections::{HashMap, VecDeque};

use serde_json::Value;

use crate::callback_shapes::{self, ApprovalOffer};
use crate::error::CallbackFault;

/// How many answered callback ids are remembered (so a late duplicate is told apart from an
/// unknown id).
const RESOLVED_MEMORY: usize = 4096;

/// The kind of a callback, from `detail.kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackKind {
    /// Permission to run a tool (answer: `{type:"approval", decision:{type}}`).
    Approval,
    /// A question card (answer: `{type:"user_input", result:{answers}}`).
    UserInput,
    /// Sign-in for a connector; surfaced to the UI, never answered through the gate.
    ConnectorAuth,
    /// A kind this version does not know; surfaced, never answered.
    Other(String),
}

impl CallbackKind {
    /// The wire name of the kind.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Approval => "approval",
            Self::UserInput => "user_input",
            Self::ConnectorAuth => "connector_auth",
            Self::Other(name) => name,
        }
    }
}

/// One open callback, as shown to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCallback {
    /// The callback id.
    pub id: String,
    /// The session it belongs to.
    pub session_id: String,
    /// Its kind.
    pub kind: CallbackKind,
    /// The approval's tool, from `detail.effect.toolName`; absent for other kinds.
    pub tool_name: Option<String>,
}

enum Offer {
    Approval(ApprovalOffer),
    UserInput(Vec<String>),
    None,
}

struct Entry {
    session_id: String,
    kind: CallbackKind,
    tool_name: Option<String>,
    offer: Offer,
    answered: bool,
}

/// All callbacks the gate knows about.
pub(crate) struct CallbackBook {
    entries: HashMap<String, Entry>,
    answered_order: VecDeque<String>,
    max_open: usize,
}

fn invalid(detail: &str) -> CallbackFault {
    CallbackFault::InvalidRegistration {
        detail: detail.to_owned(),
    }
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

impl CallbackBook {
    pub(crate) fn new(max_open: usize) -> Self {
        Self {
            entries: HashMap::new(),
            answered_order: VecDeque::new(),
            max_open: max_open.max(1),
        }
    }

    /// Records the callback of a `callback/call` (`params.callback`). Registering an id that is
    /// still open again is harmless; an id that was already answered is refused.
    pub(crate) fn register(&mut self, params: &Value) -> Result<CallbackKind, CallbackFault> {
        let callback = params
            .get("callback")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("params.callback must be an object"))?;
        let id = callback
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid("callback.id must be a non-empty string"))?;
        let session_id = callback
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid("callback.sessionId must be a non-empty string"))?;
        let detail = callback
            .get("detail")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("callback.detail must be an object"))?;
        let kind_name = detail
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("callback.detail.kind must be a string"))?;
        let (kind, offer) = match kind_name {
            "approval" => (
                CallbackKind::Approval,
                Offer::Approval(ApprovalOffer {
                    choices: string_list(detail.get("choices")),
                    path_scopes: string_list(detail.get("pathScopeChoices")),
                }),
            ),
            "user_input" => {
                let questions = detail
                    .get("request")
                    .and_then(|r| r.get("questions"))
                    .and_then(Value::as_array)
                    .map(|qs| {
                        qs.iter()
                            .filter_map(|q| {
                                q.get("question").and_then(Value::as_str).map(str::to_owned)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (CallbackKind::UserInput, Offer::UserInput(questions))
            }
            "connector_auth" => (CallbackKind::ConnectorAuth, Offer::None),
            other => (CallbackKind::Other(other.to_owned()), Offer::None),
        };
        if let Some(existing) = self.entries.get(id) {
            return if existing.answered {
                Err(CallbackFault::DuplicateId { id: id.to_owned() })
            } else {
                Ok(existing.kind.clone())
            };
        }
        if self.open_count() >= self.max_open {
            return Err(CallbackFault::TooManyOpen {
                limit: self.max_open,
            });
        }
        self.entries.insert(
            id.to_owned(),
            Entry {
                session_id: session_id.to_owned(),
                tool_name: (kind == CallbackKind::Approval)
                    .then(|| {
                        detail
                            .get("effect")?
                            .get("toolName")?
                            .as_str()
                            .map(str::to_owned)
                    })
                    .flatten(),
                kind: kind.clone(),
                offer,
                answered: false,
            },
        );
        Ok(kind)
    }

    /// Checks the original answer without claiming the callback, before the core replaces an approval.
    pub(crate) fn validate(
        &self,
        session_id: &str,
        id: &str,
        output: &Value,
    ) -> Result<(), CallbackFault> {
        let entry = self
            .entries
            .get(id)
            .ok_or_else(|| CallbackFault::UnknownCallback { id: id.to_owned() })?;
        if entry.answered {
            return Err(CallbackFault::AlreadyResolved { id: id.to_owned() });
        }
        if entry.session_id != session_id {
            return Err(CallbackFault::WrongSession { id: id.to_owned() });
        }
        match &entry.offer {
            Offer::Approval(offer) => callback_shapes::approval(output, offer)?,
            Offer::UserInput(questions) => callback_shapes::user_input(output, questions)?,
            Offer::None => {
                return Err(CallbackFault::UnsupportedKind {
                    kind: entry.kind.as_str().to_owned(),
                });
            }
        }
        Ok(())
    }

    /// Validates an answer and, only if it is valid, marks the callback answered.
    pub(crate) fn claim(
        &mut self,
        session_id: &str,
        id: &str,
        output: &Value,
    ) -> Result<(), CallbackFault> {
        self.validate(session_id, id, output)?;
        self.entries
            .get_mut(id)
            .ok_or_else(|| CallbackFault::UnknownCallback { id: id.to_owned() })?
            .answered = true;
        self.answered_order.push_back(id.to_owned());
        self.forget_old_answered();
        Ok(())
    }

    /// Forgets one callback (the server cancelled or expired it, or its turn ended).
    pub(crate) fn close(&mut self, id: &str) {
        self.entries.remove(id);
    }

    /// Forgets every callback of a session.
    pub(crate) fn close_session(&mut self, session_id: &str) {
        self.entries.retain(|_, e| e.session_id != session_id);
    }

    /// The callbacks still waiting for an answer.
    pub(crate) fn open(&self) -> Vec<OpenCallback> {
        let mut open: Vec<OpenCallback> = self
            .entries
            .iter()
            .filter(|(_, e)| !e.answered)
            .map(|(id, e)| OpenCallback {
                id: id.clone(),
                session_id: e.session_id.clone(),
                kind: e.kind.clone(),
                tool_name: e.tool_name.clone(),
            })
            .collect();
        open.sort_by(|a, b| a.id.cmp(&b.id));
        open
    }

    fn open_count(&self) -> usize {
        self.entries.values().filter(|e| !e.answered).count()
    }

    fn forget_old_answered(&mut self) {
        while self.answered_order.len() > RESOLVED_MEMORY {
            if let Some(old) = self.answered_order.pop_front()
                && self.entries.get(&old).is_some_and(|e| e.answered)
            {
                self.entries.remove(&old);
            }
        }
    }
}
