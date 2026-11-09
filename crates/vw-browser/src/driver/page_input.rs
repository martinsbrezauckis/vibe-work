//! Checked page input and bounded retries, shared by ordinary actions and native click variants.
//! Depends on the driver and isolated page scripts; called by page action dispatch.
use super::POLL;
use crate::driver::{ActError, failed, refused};
use crate::refs::PAGE_CLOSED;
use crate::{Action, Actor, Browser, PageAnswer, scripts};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
const ACT_RETRY: Duration = Duration::from_secs(2);
impl Browser {
    /// Runs the act script, polling moving targets and active entrance animations for at most [`ACT_RETRY`].
    pub(super) async fn page_act(
        &self,
        session_id: &str,
        action: &Action,
        consent: Option<&Value>,
        by: Actor,
        native_sent: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<PageAnswer, ActError> {
        self.check_ref(session_id, action)?;
        if let Action::ClickAt { reference, x, y } = action.pointer_base() {
            return self
                .coordinate_click(
                    session_id,
                    reference,
                    ((*x, *y), action.click_options()),
                    by,
                    native_sent,
                    consent,
                )
                .await;
        }
        self.page_element_act(session_id, action, consent, by, native_sent)
            .await
    }

    async fn page_element_act(
        &self,
        session_id: &str,
        action: &Action,
        consent: Option<&Value>,
        by: Actor,
        native_sent: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<PageAnswer, ActError> {
        let mount = self.check_ref(session_id, action)?;
        let page = if mount.is_some() {
            self.service
                .page_accessor(session_id)
                .ok_or_else(|| refused(PAGE_CLOSED))?
        } else {
            self.page(session_id).await.map_err(refused)?
        };
        let mut request = action
            .page_request(&page)
            .ok_or_else(|| failed("this action does not run in the page"))?;
        self.bind_ref_request(session_id, &mut request);
        // Submission is a separate authorized Enter after the settled field proves the requested text.
        request["submit"] = json!(false);
        request["nativeUser"] = json!(by == Actor::User);
        request["allowSend"] = json!(consent.is_none_or(|token| token["allowSend"] != false));
        request["nativeClick"] = json!(
            matches!(action, Action::Pointer { .. } | Action::Hover { .. })
                || by != Actor::User
                    && consent.is_some_and(|token| token["nativeClick"] == true)
                    && matches!(action, Action::Click { .. } | Action::Check { .. })
        );
        if let Some(consent) = consent
            && consent.get("snapshot").is_some()
        {
            request["consent"] = consent.clone();
        }
        #[cfg(feature = "e2e-script")]
        if let Some(origin) = self
            .test_consent_origin
            .read()
            .ok()
            .and_then(|slot| slot.clone())
        {
            request["testConsentOrigin"] = json!(origin);
        }
        let mut script = scripts::act(&request);
        let allowed = self.dispatch_permission_check(session_id, by);
        let start = tokio::time::Instant::now();
        loop {
            let answer: PageAnswer = self
                .run_act(
                    session_id,
                    script.clone(),
                    mount.as_ref(),
                    allowed.clone(),
                    // A native click brings its own real input; other input acts run as the person's gesture.
                    request["nativeClick"] != true && action.is_input(),
                )
                .await
                .map_err(|text| {
                    if text == PAGE_CLOSED || text.starts_with("Not run:") {
                        refused(text)
                    } else {
                        failed(text)
                    }
                })?;
            // Bound to the authorized document.
            self.reading_ref_allowed(session_id, &page, &answer.url, action.reference())
                .map_err(refused)?;
            let transient = crate::answers::transient(&answer) && start.elapsed() < ACT_RETRY;
            if transient || answer.status == "focusPending" {
                await_retry(start, &mut request, &mut script).await;
                continue;
            }
            // A controlled control may accept the change asynchronously; prove it after settling, without retrying input.
            let mut answer = if answer.status == "unmet"
                && matches!(
                    action,
                    Action::Check { .. } | Action::Select { .. } | Action::Type { .. }
                ) {
                answer
            } else {
                self.interpret_ref_answer(session_id, answer, action)?
            };
            if request["nativeClick"] == true {
                if let Some(point) = answer.native_point {
                    self.native_ref_click(
                        session_id,
                        action,
                        &request,
                        point,
                        by,
                        native_sent.clone(),
                    )
                    .await?;
                    // The ensuing receipt proves a native check's post-input state after settling.
                    answer.checked = None;
                } else if !matches!(action, Action::Check { .. }) {
                    return Err(refused(
                        "Not run: the page did not provide native click geometry",
                    ));
                }
            }
            return self
                .finish_input(session_id, action, &request, by, answer)
                .await;
        }
    }
}
/// Waits one poll before an act runs again while [`ACT_RETRY`] lasts. A datepicker field is typed only after the
/// page has seen its focus (`act.js` answers `focusPending`); a page that cannot take focus (its window is not
/// active) never sends it, so after [`ACT_RETRY`] the date is typed without it.
async fn await_retry(start: tokio::time::Instant, request: &mut Value, script: &mut String) {
    if start.elapsed() < ACT_RETRY {
        tokio::time::sleep(POLL).await;
    } else {
        request["focusWait"] = json!(false);
        *script = scripts::act(request);
    }
}
