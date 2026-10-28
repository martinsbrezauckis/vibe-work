//! Exact output shapes of callback answers, per callback kind.
//!
//! Depends on: error (`CallbackFault`), canonical. Called by: callbacks.
//! Sources: `ApprovalCallbackOutput` (`vibe/app_server/models.py:340`) and
//! `UserInputCallbackOutput` (`models.py:346`). A wrong shape can cause no reply for 30 s
//! and server exit code 1.
//! Invariants:
//! - Vibe Work never sends `approve_for_session` or `approve_permanently`: both make Vibe
//!   keep a rule of its own (in memory, or in Vibe Work's `config.toml` for every project). Every yes is sent as
//!   `approve`; the core keeps "Allow for this session" itself (`vw-core-api` `ask::grants`).
//! - Objects are closed: a key that is not part of the shape makes the answer malformed.
//! - Free text (feedback, answers) is only length-checked: newlines are normal in it.

use serde_json::{Map, Value};

use crate::error::CallbackFault;

/// The five approval decisions (`ApprovalDecisionType`).
const DECISIONS: &[&str] = &[
    "approve",
    "approve_for_session",
    "approve_permanently",
    "deny",
    "cancel_turn",
];
/// Decisions that would make Vibe keep a rule of its own; refused on the wire (module invariants).
const KEPT_BY_VIBE: &[&str] = &["approve_for_session", "approve_permanently"];
/// Decisions that grant something and may carry a path scope.
const GRANTING: &[&str] = &["approve", "approve_for_session", "approve_permanently"];
/// Path scopes (`PathGrantScope`).
const SCOPES: &[&str] = &["directory_recursive", "exact"];
/// Kinds the gate can answer; used to tell "wrong kind" from "garbage".
const ANSWERABLE: &[&str] = &["approval", "user_input"];

const MAX_FEEDBACK: usize = 10_000;
const MAX_ANSWER: usize = 20_000;
const MAX_ANSWERS: usize = 64;

/// What an approval callback offered, remembered from its `callback/call`.
#[derive(Debug, Clone, Default)]
pub(crate) struct ApprovalOffer {
    /// `detail.choices`; empty means the callback did not restrict them.
    pub(crate) choices: Vec<String>,
    /// `detail.pathScopeChoices`; empty means the callback did not restrict them.
    pub(crate) path_scopes: Vec<String>,
}

fn malformed(detail: &str) -> CallbackFault {
    CallbackFault::Malformed {
        detail: detail.to_owned(),
    }
}

fn closed_object<'a>(
    value: &'a Value,
    what: &str,
    allowed: &[&str],
) -> Result<&'a Map<String, Value>, CallbackFault> {
    let map = value
        .as_object()
        .ok_or_else(|| malformed(&format!("{what} must be an object")))?;
    match map.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(extra) => Err(malformed(&format!("{what} has an unknown field `{extra}`"))),
        None => Ok(map),
    }
}

/// Checks the `type` tag: it must equal `expected`; another answerable kind is a wrong-kind error.
fn check_tag(output: &Value, expected: &'static str) -> Result<(), CallbackFault> {
    let tag = output
        .as_object()
        .ok_or_else(|| malformed("output must be an object"))?
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("output needs a string `type`"))?;
    if tag == expected {
        Ok(())
    } else if ANSWERABLE.contains(&tag) {
        Err(CallbackFault::WrongKind {
            expected,
            got: tag.to_owned(),
        })
    } else {
        Err(malformed("output `type` is not a known answer kind"))
    }
}

/// Validates an approval answer against what the callback offered.
pub(crate) fn approval(output: &Value, offer: &ApprovalOffer) -> Result<(), CallbackFault> {
    check_tag(output, "approval")?;
    let top = closed_object(output, "output", &["type", "decision", "feedback"])?;
    let decision = top
        .get("decision")
        .ok_or_else(|| malformed("`decision` is required"))?;
    let decision = closed_object(decision, "decision", &["type", "pathScope"])?;
    let kind = decision
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("`decision.type` must be a string"))?;
    if !DECISIONS.contains(&kind) {
        return Err(malformed("`decision.type` is not a known decision"));
    }
    if KEPT_BY_VIBE.contains(&kind) {
        return Err(malformed(
            "`decision.type` would make Vibe keep a rule; Vibe Work answers once and keeps session grants itself",
        ));
    }
    if !offer.choices.is_empty() && !offer.choices.iter().any(|c| c == kind) {
        return Err(malformed(
            "`decision.type` was not offered by this callback",
        ));
    }
    match decision.get("pathScope") {
        None | Some(Value::Null) => {}
        Some(Value::String(scope)) => {
            let offered =
                offer.path_scopes.is_empty() || offer.path_scopes.iter().any(|s| s == scope);
            if !SCOPES.contains(&scope.as_str()) || !offered || !GRANTING.contains(&kind) {
                return Err(malformed("`decision.pathScope` is not valid here"));
            }
        }
        Some(_) => return Err(malformed("`decision.pathScope` must be a string")),
    }
    match top.get("feedback") {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(text)) if text.chars().count() <= MAX_FEEDBACK => Ok(()),
        Some(_) => Err(malformed(
            "`feedback` must be a string of reasonable length",
        )),
    }
}

/// Validates a question-card answer against the questions the callback asked.
pub(crate) fn user_input(output: &Value, questions: &[String]) -> Result<(), CallbackFault> {
    check_tag(output, "user_input")?;
    let top = closed_object(output, "output", &["type", "result"])?;
    let result = top
        .get("result")
        .ok_or_else(|| malformed("`result` is required"))?;
    let result = closed_object(result, "result", &["answers", "cancelled"])?;
    match result.get("cancelled") {
        None | Some(Value::Null | Value::Bool(_)) => {}
        Some(_) => return Err(malformed("`cancelled` must be a boolean")),
    }
    let answers = result
        .get("answers")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("`answers` must be an array"))?;
    let limit = if questions.is_empty() {
        MAX_ANSWERS
    } else {
        questions.len().min(MAX_ANSWERS)
    };
    if answers.len() > limit {
        return Err(malformed("more answers than questions"));
    }
    let mut seen: Vec<&str> = Vec::new();
    for answer in answers {
        let answer = closed_object(answer, "answer", &["question", "answer", "isOther"])?;
        let question = answer
            .get("question")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("`question` must be a string"))?;
        if !questions.is_empty() && !questions.iter().any(|q| q == question) {
            return Err(malformed("answer for a question that was not asked"));
        }
        if seen.contains(&question) {
            return Err(malformed("the same question is answered twice"));
        }
        seen.push(question);
        let text = answer
            .get("answer")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("`answer` must be a string"))?;
        if text.chars().count() > MAX_ANSWER {
            return Err(malformed("`answer` is too long"));
        }
        match answer.get("isOther") {
            None | Some(Value::Null | Value::Bool(_)) => {}
            Some(_) => return Err(malformed("`isOther` must be a boolean")),
        }
    }
    Ok(())
}
