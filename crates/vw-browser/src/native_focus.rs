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

pub(crate) const FOCUS_REQUEST: &str = "Bring Vibe Work to the front, then I'll try once.";
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
        mut cancel: tokio::sync::watch::Receiver<u64>,
    ) -> Result<Value, ActError> {
        if !self.sessions.queue_native_click(session) {
            return Err(refused(
                "A native click is already waiting; press Stop before another act.",
            ));
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
        tokio::spawn(async move {
            let _pending = pending;
            let _keep_mount = keep_mount;
            let sent = Arc::new(AtomicBool::new(false));
            let work = async {
                loop {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    let result = browser
                        .perform(
                            &session,
                            &action,
                            &StayInPage,
                            Some(&token),
                            Actor::Agent,
                            sent.clone(),
                        )
                        .await;
                    if !sent.load(Ordering::Acquire)
                        && result
                            .as_ref()
                            .is_err_and(|error| error.text == FOCUS_REQUEST)
                    {
                        continue;
                    }
                    return result;
                }
            };
            let result = tokio::select! {
                biased;
                _ = cancel.changed() => Err(ActError { outcome: Outcome::Stopped,
                    text: "The pending native click was canceled before completion.".to_owned() }),
                result = tokio::time::timeout(FOCUS_WAIT, work) => result.unwrap_or_else(|_| Err(refused(
                    "Not run: the window did not come to the front within 60 seconds; the pending click was canceled."
                ))),
            };
            // Session deletion cancels this wait; do not recreate its state or durable receipts.
            if browser
                .sessions
                .state(&session)
                .pending_native_click
                .is_none()
            {
                return;
            }
            let result = Self::finish_native_action(result, sent.load(Ordering::Acquire));
            browser.log(&session, Actor::Agent, &action, &target, &result);
        });
        Ok(
            json!({"Outcome": FOCUS_REQUEST, "ok": false, "pending": true,
            "expiresInSeconds": FOCUS_WAIT.as_secs(),
            "note": "No input has been sent. This authorized act will try once when the window is key. Do not repeat it; press Stop to cancel. Its result will appear in Browser actions."}),
        )
    }
}
