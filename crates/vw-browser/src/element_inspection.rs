//! Purpose-built inspection of one main-document element; no arbitrary expressions or page mutation.
//! Called by `browser_act` after shared read authorization; native mount checks and vault scrubbing bound the result.
use crate::{
    ActError, Actor, Browser,
    driver::{failed, refused},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One selector, optional literal attribute and a bounded computed-style property list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    selector: String,
    attribute: Option<String>,
    properties: Option<Vec<String>>,
}
impl Request {
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        let request: Self = serde_json::from_value(args.clone()).map_err(
            |_| "Inspection needs selector, optional attribute, and a list of CSS properties.",
        )?;
        let name = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
        };
        if request.selector.trim().is_empty()
            || request.selector.len() > 1000
            || request.attribute.as_deref().is_some_and(|s| !name(s))
            || request
                .properties
                .as_ref()
                .is_some_and(|p| p.is_empty() || p.len() > 32 || p.iter().any(|s| !name(s)))
        {
            return Err("Inspection needs a selector up to 1000 bytes, an attribute name, and at most 32 CSS property names.".into());
        }
        Ok(request)
    }
    fn script(&self, url: &str) -> String {
        let mut request = json!(self);
        request["url"] = json!(url);
        include_str!("js/inspect.js").replace("__REQUEST__", &request.to_string())
    }
}

impl Browser {
    pub(crate) async fn inspect_element(
        &self,
        session: &str,
        request: &Request,
        by: Actor,
    ) -> Result<Value, ActError> {
        let bound = self.bind_read(session, by).await.map_err(refused)?;
        if self.service.shots_blocked(session) {
            return Err(refused(
                "Element markup is unavailable after a vault fill because it could contain the password; open another page before inspecting markup.",
            ));
        }
        let mount = bound
            .read_mount
            .as_ref()
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        let page = bound.page_of(session).await.map_err(refused)?;
        let url = self
            .service
            .view(session)
            .and_then(|v| v.url)
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        bound.check_answer(session, &page, &url).map_err(refused)?;
        let text = self
            .service
            .evaluate_on_mount_checked(
                mount,
                request.script(&url),
                self.dispatch_permission_check(session, by),
            )
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        bound.check_read_mount().map_err(refused)?;
        let text = vw_secrets::scrub(text.as_bytes()).map_or(text.clone(), |(v, _)| {
            String::from_utf8_lossy(&v).into_owned()
        });
        let data: Value = crate::model::parse_answer(&text).map_err(failed)?;
        if let Some(error) = data["error"].as_str() {
            return Err(failed(error));
        }
        Ok(
            json!({"Outcome":"Element inspected.","ok":true,"action":"page.inspect","untrustedPageContent":data}),
        )
    }
}
