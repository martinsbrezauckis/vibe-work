//! User-selected browser values held briefly for a vault save.
//!
//! Depends on the page script runner and preview document checks. Called by the core browser command.
//! The value stays in a zeroizing core buffer; command answers contain only a one-use handle and safe metadata.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::driver::Browser;

/// A value read from a user's page selection or element click.
pub struct CapturedSecret {
    /// The value (trimmed, at most 4,096 characters).
    pub value: Zeroizing<String>,
    /// A generic description of the user's choice.
    pub element: String,
    /// The page origin.
    pub origin: String,
    // Retained with the captured value through native confirmation and storage.
    _keep_alive: vw_preview::BrowserSaveGuard,
}

impl std::fmt::Debug for CapturedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturedSecret")
            .field("element", &self.element)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

/// Metadata safe to answer to the window; it never includes the chosen value.
#[derive(Clone, Debug)]
pub struct SecretChoice {
    /// A one-use handle bound to the session and document.
    pub handle: String,
    /// Suggested vault name based on the page host, never the value.
    pub suggested_name: String,
    /// Page origin.
    pub page: String,
    /// Generic element description.
    pub element: String,
    /// Character count for the confirmation.
    pub value_length: usize,
}

struct Pending {
    handle: String,
    page: String,
    url: String,
    document_id: String,
    captured: CapturedSecret,
}

/// One outstanding choice per session; a new choice drops the previous zeroizing value.
#[derive(Default)]
pub(crate) struct SecretChoices {
    next: AtomicU64,
    inner: Mutex<HashMap<String, Pending>>,
}

impl SecretChoices {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn put(
        &self,
        session_id: &str,
        page: String,
        url: String,
        document_id: String,
        captured: CapturedSecret,
    ) -> SecretChoice {
        let handle = format!("choice-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let host = captured
            .origin
            .split_once("://")
            .map_or("this page", |(_, host)| host);
        let choice = SecretChoice {
            handle: handle.clone(),
            suggested_name: format!("{host} key").chars().take(80).collect(),
            page: captured.origin.clone(),
            element: captured.element.clone(),
            value_length: captured.value.chars().count(),
        };
        let mut inner = self.lock();
        inner.insert(
            session_id.to_owned(),
            Pending {
                handle,
                page,
                url,
                document_id,
                captured,
            },
        );
        choice
    }

    fn take(
        &self,
        session_id: &str,
        handle: &str,
        page: &str,
        url: &str,
        document_id: &str,
    ) -> Result<CapturedSecret, String> {
        let mut inner = self.lock();
        if !inner.get(session_id).is_some_and(|pending| {
            pending.handle == handle
                && pending.page == page
                && pending.url == url
                && pending.document_id == document_id
        }) {
            return Err("the choice expired or the page changed: select the key again".to_owned());
        }
        inner
            .remove(session_id)
            .map(|pending| pending.captured)
            .ok_or_else(|| "the choice expired: select the key again".to_owned())
    }

    fn remove(&self, session_id: &str) {
        self.lock().remove(session_id);
    }

    fn cancel(&self, session_id: &str, handle: &str) {
        let mut inner = self.lock();
        if inner
            .get(session_id)
            .is_some_and(|pending| pending.handle == handle)
        {
            inner.remove(session_id);
        }
    }
}

#[derive(Deserialize)]
struct Answer {
    status: String,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    url: String,
    #[serde(default, rename = "documentId")]
    document_id: String,
    #[serde(default)]
    element: String,
}

impl Browser {
    /// Drops a pending user choice when its browser session ends.
    pub fn clear_secret_choice(&self, session_id: &str) {
        self.secret_choices.remove(session_id);
    }

    /// Cancels only this handle; a late cancel cannot discard a newer choice.
    pub fn cancel_secret_choice(&self, session_id: &str, handle: &str) {
        self.secret_choices.cancel(session_id, handle);
    }

    /// Captures a current selection or waits for the user's element click, returning metadata only.
    ///
    /// # Errors
    /// A plain-text reason for an empty selection, cancellation, changed page, or unreadable value.
    pub async fn choose_secret(
        &self,
        session_id: &str,
        mode: &str,
    ) -> Result<SecretChoice, String> {
        if !matches!(mode, "selection" | "element" | "contextSelection") {
            return Err("choose selection or element".to_owned());
        }
        let keep_alive = self.service.browser_save_guard(session_id).await;
        let page = self
            .page(session_id)
            .await
            .map_err(|_| "the browser page is unavailable".to_owned())?;
        let mut raw: Answer = self
            .run(
                session_id,
                crate::scripts::secret_choice(&page, mode, "start"),
            )
            .await
            .map_err(|_| "the page could not complete this choice".to_owned())?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while raw.status == "pending" && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            raw = self
                .run(
                    session_id,
                    crate::scripts::secret_choice(&page, mode, "poll"),
                )
                .await
                .map_err(|_| "the page could not complete this choice".to_owned())?;
        }
        if raw.status != "captured" {
            let _ = self
                .run::<Answer>(
                    session_id,
                    crate::scripts::secret_choice(&page, mode, "cancel"),
                )
                .await;
        }
        let value = raw.value.map(Zeroizing::new);
        self.service
            .reading_allowed(session_id, &page, &raw.url)
            .map_err(|_| "the page changed: select the key again".to_owned())?;
        match raw.status.as_str() {
            "captured" => {}
            "empty" => return Err("select a key on the page first".to_owned()),
            "multipleValues" => {
                return Err(
                    "this element contains multiple values: select the key itself".to_owned(),
                );
            }
            "tooLong" => return Err("the value is longer than 4096 characters".to_owned()),
            "pending" | "cancelled" => return Err("the choice was cancelled".to_owned()),
            _ => return Err("the page changed: select the key again".to_owned()),
        }
        let value = value
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| "select a key on the page first".to_owned())?;
        let element = match raw.element.as_str() {
            "selected text" | "input" | "textarea" | "code" | "button" | "span" | "div" | "p"
            | "pre" | "td" | "dd" => raw.element,
            _ => "element".to_owned(),
        };
        let page_origin = url::Url::parse(&raw.url).map_or_else(
            |_| "this page".to_owned(),
            |url| url.origin().ascii_serialization(),
        );
        let origin = if page_origin.contains(value.as_str()) {
            "this page".to_owned()
        } else {
            page_origin.chars().take(300).collect()
        };
        let captured = CapturedSecret {
            value,
            element,
            origin,
            _keep_alive: keep_alive,
        };
        Ok(self
            .secret_choices
            .put(session_id, page, raw.url, raw.document_id, captured))
    }

    /// Consumes a user's previous choice, after verifying it still belongs to this document.
    ///
    /// # Errors
    /// A plain-text reason when the handle is stale, used, or bound to another document.
    pub async fn take_secret_choice(
        &self,
        session_id: &str,
        handle: &str,
    ) -> Result<CapturedSecret, String> {
        let page = self
            .page(session_id)
            .await
            .map_err(|_| "the browser page is unavailable".to_owned())?;
        let raw: Answer = self
            .run(
                session_id,
                crate::scripts::secret_choice(&page, "selection", "document"),
            )
            .await
            .map_err(|_| "the page changed: select the key again".to_owned())?;
        self.service
            .reading_allowed(session_id, &page, &raw.url)
            .map_err(|_| "the page changed: select the key again".to_owned())?;
        self.secret_choices
            .take(session_id, handle, &page, &raw.url, &raw.document_id)
    }
}
