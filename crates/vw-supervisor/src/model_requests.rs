//! Host-side model admission contract shared by the credential provider and Core.
//! The proxy implements it; Core supplies trusted turn events and owns resume guards.
use std::time::Duration;

use crate::SecretError;

/// A resume barrier. Dropping it without confirming the old turn ended must fail closed.
pub trait ModelRequestHold: Send {
    /// Admit fresh turns only after Core read the old turn's terminal state from Vibe.
    ///
    /// # Errors
    /// The hold expired, was stopped, or belongs to a previous server process.
    fn release(&mut self) -> Result<(), SecretError>;
}

/// In-process control of the same proxy that supplies the child's placeholder key.
/// These methods are never exposed over HTTP or to the agent's tools.
pub trait ModelRequests: Send + Sync {
    /// Hold one session before sending resume; no request from `old_turn` may be forwarded.
    ///
    /// # Errors
    /// The proxy is shutting down or cannot install the barrier.
    fn hold(
        &self,
        session: &str,
        old_turn: &str,
        old_inputs: &[String],
        timeout: Duration,
    ) -> Result<Box<dyn ModelRequestHold>, SecretError>;

    /// A fresh in-progress turn from the trusted protocol stream, with its input entry id.
    fn new_turn(&self, session: &str, turn: &str, input: Option<&str>);

    /// A user entry from the trusted protocol stream (including steering a fresh turn).
    fn user_input(&self, session: &str, turn: &str, input: &str);

    /// Stop a pending resume immediately, leaving its requests refused.
    fn stop(&self, session: &str);

    /// Refuse pending resumes before shutdown starts waiting for the child.
    fn shutdown(&self);
}
