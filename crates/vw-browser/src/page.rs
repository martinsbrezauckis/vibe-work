//! Agent navigation, page access and bounded script evaluation. Depends on preview engine calls and the
//! answer parser; called by browser reads, actions and captures. Ref scripts stay bound to their observed mount.

use std::{sync::Arc, time::Duration};

use crate::driver::{Browser, NOTHING_OPEN, Navigate, preview_text};
use crate::model::parse_answer;
use crate::refs::PAGE_CLOSED;
use crate::sessions::Actor;
use vw_preview::{EngineError, MountId, PreviewError};

/// Longest wait for a page that is between documents to answer a script.
const PAGE_SWAP_WAIT: Duration = Duration::from_secs(8);

impl Browser {
    /// An agent retries only a pending name refusal, once, through the same policy and tab.
    pub(crate) async fn navigate(
        &self,
        session_id: &str,
        url: &str,
        nav: &dyn Navigate,
        by: Actor,
    ) -> Result<(), String> {
        let _retry_owner = (by == Actor::Agent).then(|| self.service.agent_navigation(session_id));
        nav.open(session_id, url).await?;
        let Some((target, reason)) = self.navigation_refusal(session_id, Some(url)) else {
            return Ok(());
        };
        if by == Actor::Agent
            && target.chars().count() < 200
            && let Ok(parsed) = url::Url::parse(&target)
            && let Some(host) = parsed.host_str()
            && reason == format!("checking where {host} points; open it again in a moment")
        {
            if let Some(why) = self.service.check_name(host).await.refusal() {
                return Err(format!("{target} is not opened: {why}"));
            }
            nav.open(session_id, &target).await?;
            return match self.navigation_refusal(session_id, Some(&target)) {
                Some((url, why)) => Err(format!("{url} is not opened: {why}")),
                None => Ok(()),
            };
        }
        Err(format!("{target} is not opened: {reason}"))
    }

    fn navigation_refusal(
        &self,
        session_id: &str,
        requested: Option<&str>,
    ) -> Option<(String, String)> {
        let page = self.service.view(session_id)?;
        let requested = requested.and_then(|url| url::Url::parse(url).ok());
        let requested = requested.as_ref().map(url::Url::as_str);
        self.service
            .refusals(session_id)
            .into_iter()
            .rev()
            .find(|(url, reason)| {
                // Without a frame flag, only the requested/current URL or an unfinished open is top-level.
                reason != "pages cannot open new windows here"
                    // Refused navigations never commit; this settled URL has loaded since the refusal.
                    && (page.loading || page.url.as_deref() != Some(url.as_str()))
                    && (requested == Some(url.as_str())
                        || page.url.as_deref() == Some(url.as_str())
                        || (page.loading
                            && page.title.as_deref().is_none_or(str::is_empty)
                            && page.url.as_deref().is_none_or(|current| {
                                current.is_empty()
                                    || current == "about:blank"
                                    || requested.is_none_or(|asked| current == asked)
                            })))
            })
    }

    /// Evaluates and parses a script, remounting only when it has no observed-mount requirement.
    pub(crate) async fn run<T: serde::de::DeserializeOwned>(
        &self,
        session_id: &str,
        script: String,
    ) -> Result<T, String> {
        self.run_in_mount(session_id, script, None).await
    }

    /// The page environment's accessor, mounting a suspended preview first.
    pub(crate) async fn page(&self, session_id: &str) -> Result<String, String> {
        if let Some(page) = self.service.page_accessor(session_id) {
            return Ok(page);
        }
        self.service
            .evaluate(session_id, "1".to_owned())
            .await
            .map_err(|e| preview_text(&e))?;
        self.service
            .page_accessor(session_id)
            .ok_or_else(|| NOTHING_OPEN.to_owned())
    }

    /// Evaluates `script` and parses its answer; one retry after a moment when the page was mid-navigation.
    pub(crate) async fn run_in_mount<T: serde::de::DeserializeOwned>(
        &self,
        session_id: &str,
        script: String,
        mount: Option<&MountId>,
    ) -> Result<T, String> {
        self.run_admitted_in_mount(session_id, script, mount, None)
            .await
    }

    /// Rechecks host admission after every evaluation retry sleep and under the mounted-page lock.
    pub(crate) async fn run_admitted_in_mount<T: serde::de::DeserializeOwned>(
        &self,
        session_id: &str,
        script: String,
        mount: Option<&MountId>,
        allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<T, String> {
        // A page between documents answers nothing ("the page changed while it was read"): WebKitGTK reports a load
        // that a newer navigation cancelled as finished with the NEW URL, so a
        // receipt can start while the page is still on its way. Such an answer is retried until the page is there,
        // at most `PAGE_SWAP_WAIT`; any other unreadable answer once more.
        let start = tokio::time::Instant::now();
        let mut last = String::new();
        for attempt in 0.. {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            if allowed.as_ref().is_some_and(|allowed| !allowed()) {
                return Err("Not run: page ownership ended before input.".to_owned());
            }
            let admitted_mount = mount.cloned().or_else(|| {
                allowed
                    .as_ref()
                    .and_then(|_| self.service.page_mount(session_id))
            });
            if allowed.is_some() && admitted_mount.is_none() {
                return Err(PAGE_CLOSED.to_owned());
            }
            let result = match admitted_mount.as_ref() {
                Some(id) => {
                    self.service
                        .evaluate_on_mount_checked(id, script.clone(), allowed.clone())
                        .await
                }
                None => self.service.evaluate(session_id, script.clone()).await,
            };
            let raw = result.map_err(|error| {
                if let Some((url, reason)) = self.navigation_refusal(session_id, None) {
                    format!("{url} is not opened: {reason}")
                } else if mount.is_some()
                    && matches!(error, PreviewError::Engine(EngineError::NotMounted))
                {
                    PAGE_CLOSED.to_owned()
                } else {
                    preview_text(&error)
                }
            })?;
            match parse_answer(&raw) {
                Ok(value) => return Ok(value),
                Err(reason) => {
                    if let Some((url, reason)) = self.navigation_refusal(session_id, None) {
                        return Err(format!("{url} is not opened: {reason}"));
                    }
                    let swapping = reason.starts_with("the page changed while it was read");
                    last = reason;
                    if !(swapping && start.elapsed() < PAGE_SWAP_WAIT) && attempt >= 1 {
                        break;
                    }
                }
            }
        }
        Err(last)
    }
}
