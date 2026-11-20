//! The one permission gate between an agent's request and a vault value, before and after the person's consent.
//! Connected tools' keys (`mcp_keys_gate`) and commands run with a secret (`agent_tools::secret_run`) both go
//! through it, so the order of checks is written once.
//!
//! Steps, each on a record read now (never a copy):
//! 1. `check`: everything except the grant holds (agent access not paused, the request's scope, the record exists,
//!    no Never permission). The result is the record.
//! 2. `permits`: whether a grant holds for that record. Without one the caller may ask the person; after the answer
//!    the record is checked again from step 1, must still be what the dialog showed (`same_as_shown`), and must hold
//!    a grant given the answer. `confirmed` then records a lasting answer (for example an Allowed permission).
//! 3. The value is read only for a [`Granted`] record, which has no other constructor than this module's functions.
//! 4. After the value read the record is checked once more and must equal the granted one (a revoke, a Never, a
//!    pause or a change while the value was read refuses).
//!
//! A value read later than the consent ([`read`]) first checks the record again (steps 1 and 2) and requires it to
//! equal the granted one, so no value is read on a permission that changed after the person answered.
//!
//! Depends on: nothing outside this module (a [`Policy`] supplies the reads). Called by: `mcp_keys_gate`,
//! `agent_tools::secret_run`.

use std::future::Future;

/// The reads and refusals of one kind of request (a trait so each step can be tested with records that change
/// between two reads).
pub(crate) trait Policy: Sync {
    /// The current permission state of the request (what the checks compare).
    type Record: Clone + PartialEq + Send + Sync;
    /// The person's answer to the dialog.
    type Answer: Clone + Send + Sync;
    /// Why the request is refused.
    type Refusal: Send;
    /// What the value read returns.
    type Value;

    /// Step 1 on a fresh read.
    fn check(&self) -> impl Future<Output = Result<Self::Record, Self::Refusal>> + Send;
    /// Step 2: whether a grant holds for `record`, given the person's `answer` in this request (if any).
    fn permits(&self, record: &Self::Record, answer: Option<&Self::Answer>) -> bool;
    /// Whether `now` still shows the person what the dialog showed for `shown`.
    fn same_as_shown(&self, shown: &Self::Record, now: &Self::Record) -> bool;
    /// Records a lasting answer on the checked record and returns the record as it is now (default: unchanged).
    fn confirmed(
        &self,
        record: Self::Record,
        _answer: &Self::Answer,
    ) -> impl Future<Output = Result<Self::Record, Self::Refusal>> + Send {
        std::future::ready(Ok(record))
    }
    /// The refusal when no grant holds or the person said no.
    fn not_allowed(&self) -> Self::Refusal;
    /// The refusal when the record changed after it was shown or granted.
    fn changed(&self) -> Self::Refusal;
    /// Step 3: the value of a granted record.
    fn value(
        &self,
        granted: &Self::Record,
    ) -> impl Future<Output = Result<Self::Value, Self::Refusal>> + Send;
}

/// A record read now whose grant holds: the only way to a value.
pub(crate) struct Granted<P: Policy> {
    record: P::Record,
    answer: Option<P::Answer>,
}

impl<P: Policy> Granted<P> {
    /// The granted record.
    #[cfg(test)]
    pub(crate) fn record(&self) -> &P::Record {
        &self.record
    }

    /// The person's answer that granted it (`None`: a standing grant, no dialog).
    pub(crate) fn answer(&self) -> Option<&P::Answer> {
        self.answer.as_ref()
    }
}

/// Steps 1 and 2. `ask` shows the person the dialog for a checked record without a grant and returns the answer
/// (`Err`: declined, with the caller's refusal). The record is then read and checked again.
///
/// # Errors
/// The policy's refusal at the first step that fails.
pub(crate) async fn grant<P, A, F>(policy: &P, ask: A) -> Result<Granted<P>, P::Refusal>
where
    P: Policy,
    A: FnOnce(P::Record) -> F,
    F: Future<Output = Result<P::Answer, P::Refusal>>,
{
    let record = policy.check().await?;
    if policy.permits(&record, None) {
        return Ok(Granted {
            record,
            answer: None,
        });
    }
    let answer = ask(record.clone()).await?;
    let now = policy.check().await?;
    if !policy.same_as_shown(&record, &now) {
        return Err(policy.changed());
    }
    if !policy.permits(&now, Some(&answer)) {
        return Err(policy.not_allowed());
    }
    let record = policy.confirmed(now, &answer).await?;
    Ok(Granted {
        record,
        answer: Some(answer),
    })
}

/// Steps 3 and 4 right after [`grant`]: the value, then the record checked once more.
///
/// # Errors
/// The value read's refusal, or the policy's refusal when the record changed while the value was read.
pub(crate) async fn value<P: Policy>(
    policy: &P,
    granted: &Granted<P>,
) -> Result<(P::Record, P::Value), P::Refusal> {
    let value = policy.value(&granted.record).await?;
    let now = recheck(policy, granted).await?;
    Ok((now, value))
}

/// A value read later than the consent: the record is checked again first (steps 1 and 2) and must equal the
/// granted one, then [`value`].
///
/// # Errors
/// As [`value`]; also when the record changed since the grant.
pub(crate) async fn read<P: Policy>(
    policy: &P,
    granted: &Granted<P>,
) -> Result<(P::Record, P::Value), P::Refusal> {
    recheck(policy, granted).await?;
    value(policy, granted).await
}

/// The grant on a record read now; anything but the granted record refuses (also for a caller that holds a consent
/// across a wait and must know it still holds).
///
/// # Errors
/// The policy's refusal when the record changed since the grant.
pub(crate) async fn recheck<P: Policy>(
    policy: &P,
    granted: &Granted<P>,
) -> Result<P::Record, P::Refusal> {
    let now = policy.check().await?;
    if !policy.permits(&now, granted.answer.as_ref()) {
        return Err(policy.not_allowed());
    }
    if now != granted.record {
        return Err(policy.changed());
    }
    Ok(now)
}

#[cfg(test)]
#[path = "consent_gate_tests.rs"]
mod tests;
