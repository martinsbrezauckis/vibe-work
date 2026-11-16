//! Bounded main-world JavaScript with one dispatch and a separately polled result. On a page whose
//! Content-Security-Policy refuses `eval`, the expression runs as the source of the browser's own evaluation instead
//! (one expression only; the page's policy is not changed).
//! Shared core policy requires Full access; the preview service enforces document identity and vault exclusion.
use crate::{
    ActError, Actor, Browser,
    driver::{failed, refused},
};
use serde_json::{Value, json};

/// A JavaScript expression and completion deadline, without persistent preload authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    style: bool,
    expression: String,
    timeout_ms: u64,
}
impl Request {
    pub(crate) fn name(&self) -> &'static str {
        if self.style {
            "page.add_style"
        } else {
            "raw.execute"
        }
    }
    fn style(args: &Value) -> Result<Self, String> {
        let css=args["css"].as_str().filter(|s| !s.trim().is_empty() && s.len()<=16000).ok_or("Style insertion needs css up to 16000 bytes; provide a stylesheet for the current document.")?;
        let expression = format!(
            "(() => {{ const style=document.createElement('style'); style.textContent={}; (document.head || document.documentElement).appendChild(style); if (!style.sheet) throw new Error('The page did not accept this stylesheet'); return {{applied:true,rules:style.sheet.cssRules.length}}; }})()",
            json!(css)
        );
        Ok(Self {
            style: true,
            expression,
            timeout_ms: 5000,
        })
    }

    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        if args["action"] == "page.add_style" {
            return Self::style(args);
        }
        let expression = args["expression"].as_str().filter(|s| !s.trim().is_empty() && s.len() <= 16000).ok_or("Page JavaScript needs expression up to 16000 bytes; provide one expression, such as an immediately invoked function.")?.to_owned();
        let timeout_ms = args
            .get("timeoutMs")
            .map_or(Some(5000), Value::as_u64)
            .filter(|n| (100..=5000).contains(n))
            .ok_or("Page JavaScript timeoutMs must be 100–5000; use a bounded expression.")?;
        Ok(Self {
            style: false,
            expression,
            timeout_ms,
        })
    }
}
impl Browser {
    pub(crate) async fn page_script(
        &self,
        session: &str,
        request: &Request,
        by: Actor,
    ) -> Result<Value, ActError> {
        let bound = self.bind_read(session, by).await.map_err(refused)?;
        let mount = bound
            .read_mount
            .as_ref()
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        let state = self
            .service
            .load_state(session)
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        let key = format!("__page_result_{}", vw_preview::accessor_name());
        let script = include_str!("js/page_script.js").replace("__REQUEST__",&json!({"key":key,"expression":request.expression,"timeoutMs":request.timeout_ms,"url":state.url}).to_string());
        let admitted = self
            .service
            .page_script(
                mount,
                state.loads,
                script,
                true,
                self.dispatch_permission_check(session, by),
            )
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        let mut admitted: Value = crate::model::parse_answer(&admitted).map_err(failed)?;
        if admitted["evalRefused"] == true {
            admitted = self
                .page_script_inline(session, by, mount, &state, &key, request)
                .await?;
        }
        if let Some(error) = admitted["error"].as_str() {
            return Err(failed(error));
        }
        let poll = format!(
            "(() => {{ const key={}; const result=window[key]; if (!result) return JSON.stringify({{error:'The script result disappeared; read the page before continuing.'}}); if (result.done) delete window[key]; return JSON.stringify(result); }})()",
            json!(key)
        );
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_millis(request.timeout_ms + 1000);
        loop {
            let raw = self
                .service
                .page_script(
                    mount,
                    state.loads,
                    poll.clone(),
                    false,
                    self.dispatch_permission_check(session, by),
                )
                .await
                .map_err(|e| failed(crate::driver::preview_text(&e)))?;
            bound.check_read_mount().map_err(refused)?;
            let data: Value = crate::model::parse_answer(&raw).map_err(failed)?;
            if let Some(error) = data["error"].as_str() {
                return Err(failed(format!(
                    "Page JavaScript failed: {error}. Read the page before continuing; side effects may already have occurred."
                )));
            }
            if data["done"] == true {
                let text = data.to_string();
                let text = vw_secrets::scrub(text.as_bytes())
                    .map_or(text, |(v, _)| String::from_utf8_lossy(&v).into_owned());
                let data: Value = serde_json::from_str(&text).map_err(|_| {
                    failed(
                        "The script result could not be redacted; read the page before continuing.",
                    )
                })?;
                return Ok(
                    json!({"ok":true,"action":request.name(),"Outcome":"Page JavaScript completed; its result was read back.","untrustedPageContent":data}),
                );
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(failed(
                    "The page did not finish returning its script result; read the page before continuing and do not repeat side effects automatically.",
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    /// The page's policy refused `eval` before anything ran: the expression goes again as the source of the
    /// browser's own evaluation (which that policy does not cover); the page's policy itself is left as it is.
    async fn page_script_inline(
        &self,
        session: &str,
        by: Actor,
        mount: &vw_preview::MountId,
        state: &vw_preview::LoadState,
        key: &str,
        request: &Request,
    ) -> Result<Value, ActError> {
        let request_json =
            json!({"key":key,"timeoutMs":request.timeout_ms,"url":state.url}).to_string();
        let (head, tail) = include_str!("js/page_script_inline.js")
            .split_once("__EXPRESSION__")
            .unwrap_or_default();
        let inline = format!(
            "{}{}{tail}",
            head.replace("__REQUEST__", &request_json),
            request.expression
        );
        let raw = self
            .service
            .page_script(
                mount,
                state.loads,
                inline,
                true,
                self.dispatch_permission_check(session, by),
            )
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        if let Ok(answer) = crate::model::parse_answer(&raw) {
            return Ok(answer);
        }
        // No readable answer: a script that did not compile (a statement list) gives none and leaves no record on
        // the same document; anything else is reported as the failure it is, never as "did not start".
        let probe = format!("JSON.stringify({{started: {} in window}})", json!(key));
        let probed = self
            .service
            .page_script(
                mount,
                state.loads,
                probe,
                false,
                self.dispatch_permission_check(session, by),
            )
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        let probed: Value = crate::model::parse_answer(&probed).map_err(failed)?;
        if probed["started"] == false {
            return Err(failed(
                "Page JavaScript did not start: this page's Content-Security-Policy refuses eval, so the script must be one expression, such as an immediately invoked function. Read the page before trying again.",
            ));
        }
        Err(failed(
            "The page gave no readable answer to the script; read the page before continuing, and do not repeat side effects automatically.",
        ))
    }
}
