//! Reads a retained element ref directly into a zeroizing native buffer for the vault broker.
//! Uses normal document, frame, ownership and Stop checks; page values and exceptions never become receipts.
use crate::Browser;
use serde::Deserialize;
use serde_json::json;
use zeroize::Zeroizing;

/// A capture held only in native memory, with safe source metadata.
pub struct AgentSecret {
    /// The selected input value or element text.
    pub value: Zeroizing<String>,
    /// Safe page origin, never its path, query, title or element label.
    pub origin: String,
    _guard: vw_preview::BrowserSaveGuard,
}
#[derive(Deserialize)]
struct Answer {
    status: String,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    url: String,
}
impl Browser {
    fn capture_access(&self, session: &str) -> Result<(), String> {
        if self.sessions.stopped(session) {
            return Err(crate::driver::STOPPED.into());
        }
        Ok(())
    }

    /// Captures one currently retained ref without returning any observation or action log.
    /// # Errors
    /// A fixed diagnosis with an instruction for stale, empty, unreadable or inaccessible targets.
    pub async fn capture_agent_secret(
        &self,
        session: &str,
        reference: &str,
    ) -> Result<AgentSecret, String> {
        self.capture_access(session)?;
        let action = crate::act::Action::Hover {
            reference: reference.into(),
        };
        let mount = self.check_ref(session, &action).map_err(|_| {
            "The page ref is stale; call browser_read again and use its ref.".to_owned()
        })?;
        let guard = self.service.browser_save_guard(session).await;
        let page = self
            .page(session)
            .await
            .map_err(|_| "The page is unavailable; open it and read it again.".to_owned())?;
        let mut request = json!({"page":page,"ref":reference,"mode":"ref","op":"start"});
        self.bind_ref_request(session, &mut request);
        let answer: Answer = self
            .run_in_mount(
                session,
                crate::scripts::agent_secret(&request),
                mount.as_ref(),
            )
            .await
            .map_err(|_| {
                "The element could not be captured; read the page and try again.".to_owned()
            })?;
        let value = answer.value.map(Zeroizing::new);
        self.reading_ref_allowed(session, &page, &answer.url, Some(reference))
            .map_err(|_| "The page changed; read it and choose the ref again.".to_owned())?;
        self.capture_access(session)?;
        if answer.status != "captured" {
            return Err("The element has no capturable value or changed; read the page and point at the value itself.".into());
        }
        let value = value
            .filter(|v| !v.is_empty() && v.len() <= 4096)
            .ok_or("The element cannot be stored; point at one password or key.")?;
        let origin = url::Url::parse(&answer.url)
            .map(|u| u.origin().ascii_serialization())
            .ok()
            .filter(|s| !s.contains(value.as_str()))
            .unwrap_or_else(|| "this page".into());
        Ok(AgentSecret {
            value,
            origin,
            _guard: guard,
        })
    }
}
