//! The browser action loop: provider checks, Stop cancellation, execution and receipts.
//! Depends on the driver and native authorization callbacks; called by core agent tools and browser commands.
//! Authorization begins inside the Stop scope; only host-built tokens reach the synchronous page act.

use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::act::Action;
use crate::driver::{ActError, Browser, Navigate, STOPPED, refused};
use crate::sessions::{Actor, Outcome};

impl Browser {
    /// `browser_act`: one action by `by`, answered with a receipt. The action log records it either way.
    ///
    /// # Errors
    /// The refusal or failure text (`ActError`).
    pub async fn act(
        &self,
        session_id: &str,
        action: Action,
        by: Actor,
        nav: &dyn Navigate,
    ) -> Result<Value, ActError> {
        self.act_with_sending(session_id, action, by, nav, true)
            .await
    }

    /// Runs an authorized tool package; when sending is absent, POST activation returns a no-effect signal.
    ///
    /// # Errors
    /// The ordinary action failure, or [`crate::SEND_REQUIRED`] for the core's existing approval card.
    pub async fn act_with_sending(
        &self,
        session_id: &str,
        action: Action,
        by: Actor,
        nav: &dyn Navigate,
        allow_send: bool,
    ) -> Result<Value, ActError> {
        // The person's Stop is checked first: no later guard (point binding, an unresolved earlier input) may
        // answer a stopped agent with a different instruction.
        if by == Actor::Agent && self.sessions.stopped(session_id) {
            let target = self.target_text(session_id, &action);
            let error = refused(STOPPED);
            self.log(session_id, by, &action, &target, &Err(error.clone()));
            return Err(error);
        }
        let action = self.bind_point(session_id, action)?;
        let unknown_key = self.check_unknown_input(session_id, &action, by)?;
        let target = self.target_text(session_id, &action);
        if !matches!(action, Action::Dialog(_))
            && let Some(receipt) = self.dialog_receipt(session_id)
        {
            return Ok(receipt);
        }
        if let Err(error) = self.check_ref(session_id, &action) {
            self.log(session_id, by, &action, &target, &Err(error.clone()));
            return Err(error);
        }
        let state = self.sessions.state(session_id);
        if state.pending_native_click.is_some() {
            return Err(refused(crate::native_focus::CLICK_WAITING));
        }
        let _flight = self.sessions.begin(session_id, by);
        let mut cancel = self.sessions.cancel_signal(session_id);
        cancel.borrow_and_update();
        let mut authorization_token = None;
        let native_sent = Arc::new(AtomicBool::new(false));
        let result = {
            let work = Box::pin(async {
                let mut consent = if by == Actor::User {
                    None
                } else {
                    self.consent_before_action(session_id, &action, by).await?
                };
                if by == Actor::Agent && !allow_send {
                    self.send_before_action(session_id, &action, consent.as_ref())
                        .await?;
                    consent.get_or_insert_with(|| serde_json::json!({}))["allowSend"] =
                        serde_json::json!(false);
                }
                authorization_token.clone_from(&consent);
                self.perform(
                    session_id,
                    &action,
                    nav,
                    consent.as_ref(),
                    by,
                    native_sent.clone(),
                )
                .await
            });
            tokio::select! {
                result = work => result,
                receipt = self.wait_for_dialog(session_id), if !matches!(action, Action::Dialog(_)) => Ok(receipt),
                _ = cancel.changed(), if by == Actor::Agent => Err(ActError { capture_block: None, outcome: Outcome::Stopped,
                    text: if self.sessions.stopped(session_id) { STOPPED.to_owned() }
                        else { "Not run: this browser act was canceled because its turn ended or was replaced.".to_owned() } }),
            }
        };
        if by == Actor::Agent
            && !native_sent.load(Ordering::Acquire)
            && result
                .as_ref()
                .is_err_and(|error| error.text == crate::native_focus::FOCUS_REQUEST)
            && let Some(token) = authorization_token
        {
            return self
                .queue_focus_retry(session_id, action, token, target, cancel)
                .await;
        }
        if self.sessions.stopped(session_id) {
            self.service.dismiss_dialog(session_id).await;
        }
        if !allow_send
            && !native_sent.load(Ordering::Acquire)
            && result.as_ref().is_err_and(|error| {
                error.outcome == Outcome::Refused && error.text != crate::SEND_REQUIRED
            })
        {
            self.send_before_action(session_id, &action, authorization_token.as_ref())
                .await?;
        }
        let result = self.finish_unknown_input(
            session_id,
            unknown_key,
            result,
            native_sent.load(Ordering::Acquire),
        );
        if result
            .as_ref()
            .is_err_and(|error| error.text == crate::SEND_REQUIRED)
        {
            return result;
        }
        let result = self.record_action(session_id, &action, by, result).await;
        self.log(session_id, by, &action, &target, &result);
        result
    }

    pub(crate) fn finish_native_action(
        mut result: Result<Value, ActError>,
        sent: bool,
    ) -> Result<Value, ActError> {
        if sent {
            match &mut result {
                Ok(_) => {}
                Err(error) => {
                    if error.outcome == Outcome::Refused {
                        error.outcome = Outcome::Failed;
                    }
                    // The fixed sentence comes first: the MCP layer passes a failure through unchanged only when it
                    // starts with it, so the act's own reason (which can quote the page) never decides that label.
                    // A reason written as a refusal ("Not run: ...") is not true any more once input was sent.
                    let reason = error.text.trim();
                    let reason = reason.strip_prefix("Not run: ").unwrap_or(reason);
                    error.text = if reason.is_empty() {
                        format!("{POST_INPUT_FAILED} {POST_INPUT_ADVICE}")
                    } else {
                        format!("{POST_INPUT_FAILED} {reason} {POST_INPUT_ADVICE}")
                    };
                }
            }
        }
        result
    }
}

/// First sentence of every act that failed or stopped after native input was sent. `vw-agent-tools`' MCP layer
/// matches it as an exact prefix to keep such a failure from being reported as "Not run:".
pub const POST_INPUT_FAILED: &str = "Stopped or failed after native input was sent.";

/// Why a repeat is risky after input was sent, and what to do instead.
const POST_INPUT_ADVICE: &str = "The page may have acted on it, so repeating the act could do it twice. Do not repeat the act; check the Browser tab.";
