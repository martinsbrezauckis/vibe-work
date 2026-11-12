//! `browser_act upload`: attaches files the host read from the chat's folders to a page's file field, the way a
//! person's file picker would: the field's `files` hold them and the page gets `input` and `change`. The bytes reach
//! the page's private script world in chunks (bounded script size), then one act sets them.
//!
//! Also the receipt note for a file picker the page opened during an agent act (the engine never shows it).
//!
//! Depends on: `driver` (act admission, refs, frames), `act.js` (`uploadChunk`, `upload`). Called by:
//! `driver::perform`.

use base64::Engine as _;
use serde_json::{Value, json};

use crate::act::{Action, PageAnswer};
use crate::driver::{ActError, Browser, failed, refused};
use crate::refs::PAGE_CLOSED;
use crate::scripts;
use crate::sessions::Actor;

/// Bytes per chunk sent into the page (base64 stays well under a script's size limit).
pub(crate) const CHUNK: usize = 768 * 1024;

/// The receipt note when the page opened its file picker during this act.
pub(crate) fn picker_note(reference: Option<&str>) -> String {
    format!(
        "The page opened its file picker. It never opens for the agent: this field takes a file, which you attach \
         with browser_act upload {{\"ref\": \"{}\", \"paths\": [a file in this chat's folders]}}.",
        reference.unwrap_or("<the file field's ref>")
    )
}

impl Browser {
    /// Stages the action's files in the field's document, then sets them on the field.
    pub(crate) async fn upload(
        &self,
        session_id: &str,
        action: &Action,
        consent: Option<&Value>,
        by: Actor,
    ) -> Result<PageAnswer, ActError> {
        let Action::Upload {
            reference,
            paths,
            files,
        } = action
        else {
            return Err(failed("not an upload"));
        };
        if files.is_empty() && !paths.is_empty() {
            return Err(refused(
                "Not run: no file was loaded for this upload. Name files in this chat's folders with \"paths\".",
            ));
        }
        let mount = self.check_ref(session_id, action)?;
        let page = self
            .service
            .page_accessor(session_id)
            .ok_or_else(|| refused(PAGE_CLOSED))?;
        let allowed = self.dispatch_permission_check(session_id, by);
        let token = format!("upload-{}", crate::point_receipt::next_token());
        let page_error = |text: String| {
            if text == PAGE_CLOSED || text.starts_with("Not run:") {
                refused(text)
            } else {
                failed(text)
            }
        };
        for (file, upload) in files.iter().enumerate() {
            for (part, bytes) in upload.data.chunks(CHUNK).enumerate() {
                let request = json!({"page": page, "action": "uploadChunk", "ref": reference,
                    "token": token, "file": file, "part": part,
                    "data": base64::engine::general_purpose::STANDARD.encode(bytes)});
                let answer: PageAnswer = self
                    .run_act(
                        session_id,
                        scripts::act(&request),
                        mount.as_ref(),
                        allowed.clone(),
                        false,
                    )
                    .await
                    .map_err(page_error)?;
                if answer.status != "done" {
                    return crate::answers::interpret(answer, action);
                }
            }
        }
        let mut request = action
            .page_request(&page)
            .ok_or_else(|| failed("this action does not run in the page"))?;
        self.bind_ref_request(session_id, &mut request);
        request["token"] = json!(token);
        request["nativeUser"] = json!(by == Actor::User);
        if let Some(consent) = consent
            && consent.get("snapshot").is_some()
        {
            request["consent"] = consent.clone();
        }
        let answer: PageAnswer = self
            .run_act(
                session_id,
                scripts::act(&request),
                mount.as_ref(),
                allowed,
                false,
            )
            .await
            .map_err(page_error)?;
        self.reading_ref_allowed(session_id, &page, &answer.url, action.reference())
            .map_err(refused)?;
        self.interpret_ref_answer(session_id, answer, action)
    }
}
