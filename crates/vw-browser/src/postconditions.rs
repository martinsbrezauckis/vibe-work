//! Reads requested control states after settling and validates text before submitting it.
//! Depends on the mounted act script and answer interpreter; called by the driver before success receipts.
//! Verification never repeats a mutation; text is submitted only after the field accepted it.

use serde_json::{Value, json};

use crate::act::{Action, PageAnswer};
use crate::driver::{ActError, Browser, failed, refused};
use crate::refs::PAGE_CLOSED;
use crate::{answers, scripts, sessions::Actor};

impl Browser {
    pub(crate) async fn prove_receipt(
        &self,
        session: &str,
        action: &Action,
        result: &mut Value,
        target: &mut Value,
    ) -> Result<(), ActError> {
        if matches!(
            action,
            Action::ScrollMotion(_)
                | Action::Control(_)
                | Action::Check { .. }
                | Action::Select { .. }
                | Action::Upload { .. }
                | Action::Type { submit: false, .. }
        ) {
            let proved = self.prove_state(session, action).await?;
            if let Some(files) = proved.uploaded {
                result["files"] = files;
            }
            if let Some(scroll) = proved.scroll {
                result["scroll"] = scroll;
            }
            if let Some(checked) = proved.checked {
                result["checked"] = json!(checked);
            }
            if let Some(value) = proved.value {
                target["valueNow"] = json!(value);
            }
        }
        Ok(())
    }

    pub(crate) async fn prove_state(
        &self,
        session: &str,
        action: &Action,
    ) -> Result<PageAnswer, ActError> {
        let mount = if action.reference().is_some() {
            self.ref_mount(session)?
        } else {
            self.service
                .page_mount(session)
                .ok_or_else(|| refused(PAGE_CLOSED))?
        };
        let page = self
            .service
            .page_accessor(session)
            .ok_or_else(|| refused(PAGE_CLOSED))?;
        let mut request = action
            .page_request(&page)
            .ok_or_else(|| failed("this action has no requested control state"))?;
        self.bind_ref_request(session, &mut request);
        request["postconditionOnly"] = json!(true);
        let answer: PageAnswer = self
            .run_in_mount(session, scripts::act(&request), Some(&mount))
            .await
            .map_err(failed)?;
        self.check_read_mount()
            .and_then(|()| {
                self.reading_ref_allowed(session, &page, &answer.url, action.reference())
            })
            .map_err(failed)?;
        answers::interpret(answer, action)
    }

    pub(crate) async fn submit_checked_text(
        &self,
        session: &str,
        action: &Action,
        request: &Value,
        by: Actor,
    ) -> Result<PageAnswer, ActError> {
        let loads = self
            .service
            .load_state(session)
            .map_or(0, |state| state.loads);
        self.wait_quiet(session, loads, action.reference()).await;
        let mut proved = self.prove_state(session, action).await?;
        let mount = self.ref_mount(session)?;
        let mut press = request.clone();
        press["action"] = json!("press");
        press["key"] = json!("Enter");
        press["expectedText"] = request["text"].clone();
        press["nativeKeyboard"] = json!(true);
        press["keyboard"] = json!({"key":"Enter","modifiers":[],"expectedText":request["text"]});
        let answer: PageAnswer = self
            .run_act(
                session,
                scripts::act(&press),
                Some(&mount),
                self.dispatch_permission_check(session, by),
                false,
            )
            .await
            .map_err(failed)?;
        self.check_read_mount()
            .and_then(|()| {
                self.reading_ref_allowed(
                    session,
                    request["page"].as_str().unwrap_or(""),
                    &answer.url,
                    action.reference(),
                )
            })
            .map_err(failed)?;
        let answer = answers::interpret(answer, action)?;
        let point = answer.native_point.ok_or_else(|| {
            refused(
                "The field could not prepare native Enter. Read the field again before submitting.",
            )
        })?;
        let key = Action::Press {
            key: "Enter".into(),
            reference: action.reference().map(str::to_owned),
        };
        let native = self
            .native_ref_click(
                session,
                &key,
                &press,
                point,
                by,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )
            .await?;
        proved.effect = native.effect;
        Ok(proved)
    }
}
