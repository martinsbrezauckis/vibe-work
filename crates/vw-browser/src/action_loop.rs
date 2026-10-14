//! The browser action loop: provider checks, Stop cancellation, execution and receipts.
//! Depends on the driver and native authorization callbacks; called by core agent tools and browser commands.
//! Authorization begins inside the Stop scope; only host-built tokens reach the synchronous page act.

use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::act::Action;
use crate::driver::{ActError, Browser, LOCAL_NETWORK, Navigate, STOPPED, refused};
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
        let target = self.target_text(session_id, &action);
        if by == Actor::Agent && self.sessions.stopped(session_id) {
            let error = refused(STOPPED);
            self.log(session_id, by, &action, &target, &Err(error.clone()));
            return Err(error);
        }
        if by == Actor::Agent
            && self.on_local_network(session_id)
            && !matches!(action, Action::Navigate { .. })
        {
            let error = refused(LOCAL_NETWORK);
            self.log(session_id, by, &action, &target, &Err(error.clone()));
            return Err(error);
        }
        if let Err(error) = self.check_ref(session_id, &action) {
            self.log(session_id, by, &action, &target, &Err(error.clone()));
            return Err(error);
        }
        if self
            .sessions
            .state(session_id)
            .pending_native_click
            .is_some()
        {
            return Err(refused(
                "A native click is already waiting. Bring Vibe Work to the front or press Stop before another act.",
            ));
        }
        let _flight = self.sessions.begin(session_id, by);
        let mut cancel = self.sessions.cancel_signal(session_id);
        cancel.borrow_and_update();
        let mut authorization_token = None;
        let native_sent = Arc::new(AtomicBool::new(false));
        let result = {
            let work = async {
                let consent = if by == Actor::User {
                    None
                } else {
                    self.consent_before_action(session_id, &action, by).await?
                };
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
            };
            if by == Actor::Agent {
                tokio::select! {
                    result = work => result,
                    _ = cancel.changed() => Err(ActError { outcome: Outcome::Stopped,
                        text: if self.sessions.stopped(session_id) { STOPPED.to_owned() }
                            else { "Not run: this browser act was canceled because its turn ended or was replaced.".to_owned() } }),
                }
            } else {
                work.await
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
        let result = Self::finish_native_action(result, native_sent.load(Ordering::Acquire));
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
                    "Stopped or failed after native mouse input was sent. Do not repeat the act; check the Browser tab.".clone_into(&mut error.text);
                }
            }
        }
        result
    }
}
