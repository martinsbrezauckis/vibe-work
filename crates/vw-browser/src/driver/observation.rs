//! Browser reads, action logs and post-action receipts. Uses the driver and observation model; called by the public read and action paths.
use super::{ActError, Baseline, Browser, HEADLINE_KEY, failed, preview_text};
use crate::sessions::Outcome;
use crate::{Action, Actor, ReadOptions, observe, receipt};
use serde_json::{Value, json};
use vw_preview::Shot;

impl Browser {
    /// `browser_read`: the observation, and a screenshot when asked, under the current page authority.
    ///
    /// # Errors
    /// A plain-text reason.
    pub async fn read(
        &self,
        session_id: &str,
        mut options: ReadOptions,
        screenshot: bool,
        by: Actor,
    ) -> Result<(Value, Option<Shot>), String> {
        if options.choice_cursor.is_some() {
            if options.find.is_some()
                || options.text_cursor.is_some()
                || options.text_page.is_some()
            {
                return Err(
                    "choiceCursor reads choices; omit find, textCursor and textPage".into(),
                );
            }
            options.max_refs = 0;
        }
        if let Some(receipt) = self.dialog_receipt(session_id) {
            return Ok((receipt, None));
        }
        let clamped =
            options.max_bytes > observe::MAX_BUDGET || options.max_refs > observe::MAX_OUTPUT_REFS;
        options.max_refs = options.max_refs.min(observe::MAX_OUTPUT_REFS);
        let find = options.find.clone();
        let selector = options.selector.clone();
        let (preview, budget) = (
            options.text_page.is_none() && options.text_cursor.is_none(),
            options
                .max_bytes
                .clamp(observe::MIN_BUDGET, observe::MAX_BUDGET),
        );
        // A page loading again (a front page refreshing itself, a released tab reloaded here) is read once loaded.
        self.page(session_id).await?;
        self.wait_loaded(session_id).await;
        // The wait can outlast the page: the user may have navigated this tab meanwhile.

        let bound = self.bind_read(session_id, by).await?;
        let (mut model, verdict, snapshot) = bound.observe(session_id, options).await?;
        let mut host = self.host(session_id);
        if clamped {
            host["outputLimit"] =
                json!("Requested maxBytes/maxRefs clamped to inline limits: 4000 bytes, 32 refs.");
        }
        if screenshot {
            Self::label_refs(&mut model);
        }
        let mut answer = match find.as_deref() {
            Some(query) => observe::found(&model, query, &verdict, &snapshot, host, budget),
            None => observe::answer(&model, &verdict, &snapshot, host, preview, budget),
        };
        if let Some(selector) = selector {
            answer["matches"] = json!({"count":model.selector_matches.unwrap_or(0),
                "complete":!model.truncated && model.unread_frames.is_empty() && model.cross_origin_frames == 0,
                "index":selector.index});
            answer["nextStep"] = json!(
                "Matches are in document order, main document before frames. Use selector.index (0 to 199) for one match, or narrow the selector. A partial scan is not a complete count."
            );
            crate::observe::fit(&mut answer, budget);
        }
        #[cfg(feature = "e2e-script")]
        self.before_image().await;
        let shot = if screenshot {
            Some(
                self.service
                    .screenshot(session_id)
                    .await
                    .map_err(|e| preview_text(&e))?,
            )
        } else {
            None
        };
        // The image must show the observed page: the same mount and page class (the read binding), and the same
        // document the refs came from (a navigation inside the mount during the shot).
        bound.check_read_mount()?;
        if screenshot {
            self.ref_mount(session_id).map_err(|error| error.text)?;
        }
        // Labels go on the agent's copy only, after the image is known to show the observed page.
        let shot = shot
            .map(|shot| crate::labels::annotate(shot, &model, &answer))
            .transpose()?;
        Ok((answer, shot))
    }

    pub(crate) fn target_text(&self, session_id: &str, action: &Action) -> String {
        if let Action::Navigate { url } = action {
            return url.clone();
        }
        let seen = self.sessions.seen(session_id);
        action.reference().map_or_else(String::new, |r| {
            seen.and_then(|s| s.names.get(r).cloned())
                .map_or_else(|| r.to_owned(), |(role, name)| format!("{role} '{name}'"))
        })
    }

    pub(crate) fn log(
        &self,
        session_id: &str,
        by: Actor,
        action: &Action,
        target: &str,
        result: &Result<Value, ActError>,
    ) {
        let (outcome, detail) = match result {
            Ok(receipt) => (
                Outcome::Done,
                receipt["change"]["summary"]
                    .as_str()
                    .unwrap_or("done")
                    .to_owned(),
            ),
            Err(error) => (error.outcome, error.text.clone()),
        };
        self.sessions
            .end(session_id, by, action.name(), target, outcome, &detail);
    }

    /// Settles, observes again and builds the receipt of an action against `base` (the page before it).
    pub(super) async fn receipt(
        &self,
        session_id: &str,
        action: &Action,
        base: Baseline,
        mut result: Value,
        mut page_target: Value,
    ) -> Result<Value, ActError> {
        let Baseline {
            before,
            reference,
            loads,
            console: console0,
            refused: refused0,
        } = base;
        let before = Self::receipt_baseline(before, reference.as_deref());
        self.await_action_navigation(session_id, action, loads, refused0)
            .await;
        let dialogs = self.settle(session_id, loads, reference.as_deref()).await;
        if let Some(outcome) = self.navigation_outcome(session_id, loads, refused0, &result) {
            return outcome;
        }
        self.prove_receipt(session_id, action, &mut result, &mut page_target)
            .await?;
        let after = self.observe_receipt(session_id, reference.as_deref()).await;
        if let Some(outcome) = self.navigation_outcome(session_id, loads, refused0, &result) {
            return outcome;
        }
        let mut receipt = self.host(session_id);
        receipt["result"] = result;
        let (after_model, verdict, snapshot) = after.map_err(failed)?;
        let (facts, changed) = receipt::diff(before.as_ref(), &after_model);
        receipt["change"] = facts;
        if receipt["result"]["scroll"]["moved"] == true
            && receipt["change"]["summary"] == "no visible change"
        {
            receipt["change"]["summary"] = json!("the intended scroll container moved");
        }
        let refused_now: Vec<Value> = self
            .service
            .refusals_since(session_id, refused0)
            .into_iter()
            .map(|(url, reason)| json!({"url": url, "reason": reason}))
            .collect();
        let errors: Vec<Value> = match console0 {
            Some(mark) => self
                .service
                .console_since(session_id, mark)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.level == vw_preview::ConsoleLevel::Error || e.level.is_page_error())
                .take(5)
                .map(|e| json!(e.text.chars().take(300).collect::<String>()))
                .collect(),
            None => Vec::new(),
        };
        receipt["snapshot"] = json!(snapshot);
        // The receipt is a delta (ShellX's shape): a first look at a page carries the compact observation (refs, text
        // preview); later receipts carry what changed and only the refs that are new.
        let first_look = before.is_none();
        let mut content = if first_look {
            observe::content(&after_model, true)
        } else {
            receipt::delta(before.as_ref(), &after_model)
        };
        content["outcomeText"] = json!(receipt::outcome_text(
            &changed,
            &after_model.title,
            first_look
        ));
        content["target"] = page_target;
        content["changed"] = changed;
        if !refused_now.is_empty() {
            content["refusedNavigations"] = json!(refused_now);
        }
        if !dialogs.is_empty() {
            content["dialogsAnswered"] = json!(dialogs);
        }
        if !errors.is_empty() {
            content["consoleErrorsSinceAction"] = json!(errors);
        }
        // Dialog messages and console errors are page-made text too: the guard looks at them.
        let extra: Vec<(String, String)> = dialogs
            .iter()
            .filter_map(|d| {
                d["message"]
                    .as_str()
                    .map(|m| ("dialog".to_owned(), m.to_owned()))
            })
            .chain(
                errors
                    .iter()
                    .filter_map(|e| e.as_str().map(|t| ("console".to_owned(), t.to_owned()))),
            )
            .collect();
        let verdict = if extra.is_empty() {
            verdict
        } else {
            observe::verdict_with(&after_model, &extra)
        };
        receipt[HEADLINE_KEY] = json!(receipt::headline(&receipt["change"], first_look));
        // A flag the agent's code can test (code compared `Outcome` with guessed words such as
        // "Typed"); a refused or failed action is a tool error instead.
        receipt["ok"] = json!(true);
        receipt["untrustedPageContent"] = content;
        receipt["promptGuard"] = observe::guard_json(&verdict);
        receipt["note"] = json!(observe::UNTRUSTED_NOTE);
        observe::fit(&mut receipt, observe::READ_BUDGET);
        Ok(receipt)
    }
}
