//! No-input send classification for Ask browser activations, repeated at scripted/native dispatch.
//! Depends on retained refs and the ordinary act script; the core turns the internal signal into its existing card.
use crate::{
    Action, Browser, PageAnswer,
    driver::{ActError, refused},
    scripts,
};
use serde_json::{Value, json};

/// Internal no-effect signal for the core's pre-tool hook; never returned as a model refusal.
pub const SEND_REQUIRED: &str = "browser send approval required";

impl Browser {
    pub(crate) async fn send_before_action(
        &self,
        session: &str,
        action: &Action,
        consent: Option<&Value>,
    ) -> Result<(), ActError> {
        let action = action.pointer_base();
        if !matches!(
            action,
            Action::Click { .. }
                | Action::ClickAt { .. }
                | Action::Check { .. }
                | Action::Press { .. }
        ) {
            return Ok(());
        }
        let mount = self.check_ref(session, action)?;
        let page = self
            .service
            .page_accessor(session)
            .ok_or_else(|| refused(crate::refs::PAGE_CLOSED))?;
        let inspect = if let Action::ClickAt { reference, .. } = action {
            Action::Click {
                reference: reference.clone(),
            }
        } else {
            action.clone()
        };
        let Some(mut request) = inspect.page_request(&page) else {
            return Ok(());
        };
        self.bind_ref_request(session, &mut request);
        request["permissionCheckOnly"] = json!(true);
        request["allowSend"] = json!(false);
        if let Some(consent) = consent.filter(|token| token.get("snapshot").is_some()) {
            request["consent"] = consent.clone();
        }
        let answer: PageAnswer = self
            .run_in_mount(session, scripts::act(&request), mount.as_ref())
            .await
            .map_err(refused)?;
        if answer.status == "sendRequired" {
            Err(refused(SEND_REQUIRED))
        } else {
            Ok(())
        }
    }
}
