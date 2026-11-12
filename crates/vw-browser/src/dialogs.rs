//! Native page dialog actions and interrupt receipts, independent of a blocked page's JavaScript.
//! The driver uses this for reads/answers; HTTP Basic credentials enter only through the core's vault writer.
//! Depends on: the browser driver and preview dialog service.
//! Called by: browser action dispatch.
use crate::{
    ActError, Actor, Browser,
    driver::{failed, refused},
};
use serde_json::{Value, json};

/// A bounded native dialog operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    action: String,
    id: Option<String>,
    text: Option<String>,
}
impl Request {
    /// Checks the action and binds answers to the id returned by a native dialog read.
    ///
    /// # Errors
    /// Missing id, invalid action, or unbounded text.
    pub fn parse(args: &Value) -> Result<Self, String> {
        let action = args["action"].as_str().unwrap_or_default();
        if !matches!(
            action,
            "page.dialogs" | "page.dialog.accept" | "page.dialog.dismiss" | "page.dialog.prompt"
        ) {
            return Err("Unknown dialog action.".into());
        }
        let id = args
            .get("dialogId")
            .map(|id| {
                id.as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 256)
                    .map(str::to_owned)
                    .ok_or("dialogId must be the id from page.dialogs.")
            })
            .transpose()?;
        if action != "page.dialogs" && id.is_none() {
            return Err("Read page.dialogs and answer its dialogId; the page may have replaced a previous dialog.".into());
        }
        let text = if action == "page.dialog.prompt" {
            Some(
                args["text"]
                    .as_str()
                    .filter(|s| s.len() <= 60000)
                    .ok_or("Prompt text must be text of at most 60000 bytes.")?
                    .to_owned(),
            )
        } else {
            None
        };
        Ok(Self {
            action: action.into(),
            id,
            text,
        })
    }
    /// Chrome-compatible action name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self.action.as_str() {
            "page.dialog.accept" => "page.dialog.accept",
            "page.dialog.dismiss" => "page.dialog.dismiss",
            "page.dialog.prompt" => "page.dialog.prompt",
            _ => "page.dialogs",
        }
    }
}

impl Browser {
    pub(crate) fn dialog_receipt(&self, session: &str) -> Option<Value> {
        self.service.pending_dialog(session).map(|dialog| receipt("Page is waiting for a dialog answer. The triggering input may already have acted; do not repeat it. Read the dialog and answer its dialogId before continuing.", dialog))
    }

    pub(crate) async fn wait_for_dialog(&self, session: &str) -> Value {
        loop {
            if let Some(receipt) = self.dialog_receipt(session) {
                return receipt;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    pub(crate) async fn dialog_action(
        &self,
        session: &str,
        request: &Request,
        by: Actor,
    ) -> Result<Value, ActError> {
        if by == Actor::Agent
            && self
                .dispatch_permission_check(session, by)
                .is_some_and(|check| !check())
        {
            return Err(refused(crate::STOPPED));
        }
        let dialog = self.service.pending_dialog(session);
        if request.action == "page.dialogs" {
            return Ok(receipt(
                "Read the current native page dialog.",
                dialog.unwrap_or(Value::Null),
            ));
        }
        let dialog =
            dialog.ok_or_else(|| refused("No dialog is open. Read the page before continuing."))?;
        let id = request.id.as_deref().unwrap_or_default();
        if dialog["dialogId"] != id {
            return Err(refused(
                "The observed dialog changed. Read page.dialogs again before answering.",
            ));
        }
        let accept = request.action != "page.dialog.dismiss";
        if accept && dialog["type"] == "auth" {
            return Err(refused(
                "HTTP sign-in requires auth.basic with a vault login; ask the user to save or choose that login.",
            ));
        }
        if accept && dialog["type"] == "prompt" && request.text.is_none() {
            return Err(refused(
                "This is a prompt. Use page.dialog.prompt with its dialogId and text, or page.dialog.dismiss.",
            ));
        }
        if request.text.is_some() && dialog["type"] != "prompt" {
            return Err(refused(
                "This dialog is not a prompt. Read page.dialogs and choose accept or dismiss.",
            ));
        }
        let mount = self
            .service
            .page_mount(session)
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        self.service
            .answer_dialog(
                &mount,
                id,
                vw_preview::DialogAnswer {
                    accept,
                    text: request.text.clone(),
                    credentials: None,
                },
            )
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        Ok(
            json!({"Outcome":"Answered the native page dialog.","action":request.name(),"handled":true,"dialogId":id,"accepted":accept,"untrustedPageContent":{"dialog":safe(dialog)}}),
        )
    }
}
fn safe(value: Value) -> Value {
    let text = value.to_string();
    vw_secrets::scrub(text.as_bytes())
        .map(|(v, _)| serde_json::from_slice(&v).unwrap_or_else(|_| json!({"redacted":true})))
        .unwrap_or(value)
}
fn receipt(outcome: &str, dialog: Value) -> Value {
    json!({"Outcome":outcome,"action":"page.dialogs","untrustedPageContent":{"dialog":safe(dialog)}})
}
