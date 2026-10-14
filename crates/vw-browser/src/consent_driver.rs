//! Live provider consent preflight and private fixture mapping for the browser action loop.
//!
//! Depends on: the browser driver, consent classifier, and the page probe. Called by: `Browser::act` and test setup.
//! Invariant: a wider grant waits for native confirmation before the action script receives a document-bound token.

use crate::act::Action;
use crate::consent::{self, ConsentConfirmer, ConsentPage, ConsentRequest, Decision, Provider};
use crate::driver::{ActError, Browser, refused};
use crate::refs::PAGE_CLOSED;
use crate::scripts;
use crate::sessions::Actor;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConsentProbe {
    url: String,
    title: String,
    client: String,
    scopes: Vec<String>,
    account_chooser: bool,
    snapshot: String,
    #[serde(default)]
    stale: bool,
    #[serde(default)]
    oversized: bool,
}

impl Browser {
    /// Installs the native confirmation callback shared by this browser's clones.
    pub fn set_consent_confirmer(&self, confirmer: Arc<dyn ConsentConfirmer>) {
        if let Ok(mut slot) = self.consent_confirmer.write() {
            *slot = Some(confirmer);
        }
    }

    /// Maps one explicit loopback origin to a Google authorization fixture in private e2e builds.
    ///
    /// # Errors
    /// Refuses non-loopback or portless origins.
    #[cfg(feature = "e2e-script")]
    pub fn set_test_consent_provider(&self, origin: &str) -> Result<(), String> {
        let url = url::Url::parse(origin).map_err(|_| "invalid fixture origin")?;
        if url.scheme() != "http"
            || !matches!(
                url.host_str(),
                Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
            )
            || url.port().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("fixture must be one exact loopback origin with a port".to_owned());
        }
        let mut slot = self
            .test_consent_origin
            .write()
            .map_err(|_| "fixture lock")?;
        *slot = Some(url.origin().ascii_serialization());
        Ok(())
    }

    pub(crate) async fn consent_before_action(
        &self,
        session_id: &str,
        action: &Action,
        by: Actor,
    ) -> Result<Option<Value>, ActError> {
        if matches!(
            action,
            Action::Navigate { .. }
                | Action::Back
                | Action::Forward
                | Action::Reload
                | Action::Wait { .. }
                | Action::Hover { .. }
                | Action::Scroll { .. }
        ) {
            return Ok(None);
        }
        let mount = self.check_ref(session_id, action)?;
        let page = if mount.is_some() {
            self.service
                .page_accessor(session_id)
                .ok_or_else(|| refused(PAGE_CLOSED))?
        } else {
            self.page(session_id).await.map_err(refused)?
        };
        let probe: ConsentProbe = self
            .run_in_mount(
                session_id,
                scripts::consent(&page, action.reference()),
                mount.as_ref(),
            )
            .await
            .map_err(refused)?;
        self.service
            .reading_allowed(session_id, &page, &probe.url)
            .map_err(refused)?;
        if probe.stale {
            return Err(refused("the authorization page changed: read it again"));
        }
        let page_info = ConsentPage {
            url: probe.url.clone(),
            title: probe.title.clone(),
            client: probe.client.clone(),
            scopes: probe.scopes.clone(),
            account_chooser: probe.account_chooser
                && matches!(action, Action::Click { .. } | Action::Press { .. }),
        };
        #[allow(unused_mut)]
        let mut decision = consent::classify(&page_info);
        #[cfg(feature = "e2e-script")]
        if decision == Decision::Other && self.fixture_matches(&probe.url) {
            let fake = ConsentPage {
                url: format!(
                    "https://accounts.google.com{}",
                    url::Url::parse(&probe.url)
                        .map_err(|_| refused("invalid authorization URL"))?
                        .path()
                ),
                ..page_info.clone()
            };
            decision = consent::classify(&fake);
        }
        if decision != Decision::Other
            && (probe.oversized
                || probe.scopes.iter().map(String::len).sum::<usize>() > 4_000
                || consent::scope_tokens(&probe.scopes).len() > 40)
        {
            return Err(refused(
                "Not run: the permission list is too long to verify",
            ));
        }
        if decision != Decision::Other
            && matches!(action, Action::Press { reference: None, key } if matches!(key.as_str(), "Enter" | "Space" | " "))
        {
            return Err(refused(
                "this authorization action needs a ref from browser_read",
            ));
        }
        if let Decision::Wider(provider) = decision {
            self.confirm_wider(session_id, provider, by, &probe).await?;
        }
        if decision == Decision::Other {
            return Ok(None);
        }
        Ok(Some(
            json!({"snapshot": probe.snapshot, "ref": action.reference().unwrap_or(""),
                "nativeClick": matches!(decision, Decision::Wider(_))}),
        ))
    }

    async fn confirm_wider(
        &self,
        session_id: &str,
        provider: Provider,
        by: Actor,
        probe: &ConsentProbe,
    ) -> Result<(), ActError> {
        let confirmer = self
            .consent_confirmer
            .read()
            .ok()
            .and_then(|slot| slot.clone())
            .ok_or_else(|| refused("Not run: native access confirmation is unavailable"))?;
        let service = probe.client.trim().chars().take(120).collect::<String>();
        let service = if service.is_empty() {
            "this service"
        } else {
            &service
        };
        let request = ConsentRequest {
            session_id: session_id.to_owned(),
            actor: by,
            provider,
            title: probe.title.clone(),
            client: probe.client.clone(),
            scopes: probe.scopes.clone(),
        };
        let _authorization_wait = self.sessions.authorization_wait(session_id, by);
        if !confirmer.confirm(request).await {
            return Err(refused(format!(
                "Not run: the user declined access to {service} through {}",
                provider.name(),
            )));
        }
        Ok(())
    }

    #[cfg(feature = "e2e-script")]
    fn fixture_matches(&self, address: &str) -> bool {
        let Some(configured) = self
            .test_consent_origin
            .read()
            .ok()
            .and_then(|slot| slot.clone())
        else {
            return false;
        };
        let Ok(url) = url::Url::parse(address) else {
            return false;
        };
        url.origin().ascii_serialization() == configured && url.path().starts_with("/o/oauth2/")
    }
}
