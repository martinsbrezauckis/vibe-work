//! One-use focus waits for authorized native input. Depends on the browser, original authorization and Stop signal.
//! Called by the action loop only after an OS focus refusal with no press sent; never activates a window itself.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use serde_json::{Value, json};

use crate::act::Action;
use crate::driver::{ActError, Browser, NavFuture, Navigate, refused};
use crate::sessions::{Actor, Outcome, Sessions};

pub(crate) const FOCUS_REQUEST: &str = "No input has been sent: Vibe Work needs to be in front. Ask the user to bring it forward; this click will try once.";
pub(crate) const CLICK_WAITING: &str = "A native click is already waiting for Vibe Work to come to the front. Do not repeat it; ask the user to bring the window forward or press Stop to cancel.";
const FOCUS_WAIT: Duration = Duration::from_secs(60);

struct Pending {
    sessions: Arc<Sessions>,
    session: String,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.sessions.clear_native_click(&self.session);
    }
}

struct StayInPage;
impl Navigate for StayInPage {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> NavFuture<'a> {
        Box::pin(async { Err("A pending native click cannot navigate".to_owned()) })
    }
}

impl Browser {
    pub(crate) async fn queue_focus_retry(
        &self,
        session: &str,
        action: Action,
        token: Value,
        target: String,
        cancel: tokio::sync::watch::Receiver<u64>,
    ) -> Result<Value, ActError> {
        if !self.sessions.queue_native_click(session) {
            return Err(refused(CLICK_WAITING));
        }
        let pending = Pending {
            sessions: self.sessions.clone(),
            session: session.to_owned(),
        };
        // Keep this admitted document alive even if the model finishes its turn after the pending answer.
        let keep_mount = self.service.browser_save_guard(session).await;
        // Keep the original act's cancellation generation across mount protection's await.
        if self.sessions.stopped(session) || !matches!(cancel.has_changed(), Ok(false)) {
            return Err(refused(
                "The pending native click was canceled before it could wait for focus.",
            ));
        }
        self.sessions.end(
            session,
            Actor::Agent,
            action.name(),
            &target,
            Outcome::Refused,
            FOCUS_REQUEST,
        );
        let browser = self.clone();
        let session = session.to_owned();
        let waits_for_send_check = token["allowSend"] == false;
        let work = async move {
            let _pending = pending;
            let _keep_mount = keep_mount;
            browser
                .retry_when_focused(&session, &action, &token, &target, cancel)
                .await
        };
        // The Ask hook must remain available for a newly discovered send instead of losing its card path.
        if waits_for_send_check {
            return Box::pin(work).await;
        }
        tokio::spawn(work);
        Ok(
            json!({"Outcome": FOCUS_REQUEST, "ok": false, "pending": true,
            "expiresInSeconds": FOCUS_WAIT.as_secs(),
            "note": "No input has been sent. This authorized act will try once when the window is key. Do not repeat it: it is already queued, and a repeat is refused while it waits. Ask the user to press Stop if they want to cancel. Its result will appear in Browser actions."}),
        )
    }

    async fn retry_when_focused(
        &self,
        session: &str,
        action: &Action,
        token: &Value,
        target: &str,
        mut cancel: tokio::sync::watch::Receiver<u64>,
    ) -> Result<Value, ActError> {
        let sent = Arc::new(AtomicBool::new(false));
        let work = async {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                if token["allowSend"] == false {
                    self.send_before_action(session, action, Some(token))
                        .await?;
                }
                let result = self
                    .perform(
                        session,
                        action,
                        &StayInPage,
                        Some(token),
                        Actor::Agent,
                        sent.clone(),
                    )
                    .await;
                if !sent.load(Ordering::Acquire) {
                    if result
                        .as_ref()
                        .is_err_and(|error| error.text == FOCUS_REQUEST)
                    {
                        continue;
                    }
                    if token["allowSend"] == false && result.is_err() {
                        self.send_before_action(session, action, Some(token))
                            .await?;
                    }
                }
                return result;
            }
        };
        let result = tokio::select! {
            biased;
            _ = cancel.changed() => Err(ActError { capture_block: None, outcome: Outcome::Stopped,
                text: "The pending native click was canceled before completion.".to_owned() }),
            result = tokio::time::timeout(FOCUS_WAIT, work) => result.unwrap_or_else(|_| Err(refused(
                "Not run: the window did not come to the front within 60 seconds; the pending click was canceled."
            ))),
        };
        // Session deletion cancels this wait; do not recreate its state or durable receipts.
        if self.sessions.state(session).pending_native_click.is_none() {
            return Err(refused(
                "The pending native click was canceled before completion.",
            ));
        }
        let result = Self::finish_native_action(result, sent.load(Ordering::Acquire));
        if !result
            .as_ref()
            .is_err_and(|error| error.text == crate::SEND_REQUIRED)
        {
            self.log(session, Actor::Agent, action, target, &result);
        }
        result
    }
}
