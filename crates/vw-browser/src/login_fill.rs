//! Vault fill in the agent browser: the ShellX checks against credential retargeting ("a same-origin page retargets a
//! granted credential") on the
//! session's latest observation and in the page, and the write. The vault, the grant and the dialog are the core's
//! (vw-core-api `browser_fill`); this module never sees a vault and gets a value only for [`Browser::fill_write`].
//!
//! Order (the core keeps it): [`Browser::fill_check`] (host checks on the host-held observation, then the page script
//! without a value) -> the core's login lookup, grant or dialog, password read -> [`Browser::fill_write`] (the page
//! script repeats every check in the same call that writes).
//!
//! Depends on: `driver` (page accessor, Stop, the Lan rule), `sessions` (the latest observation), `scripts`
//! (`js/fill.js`), `model`, url, zeroize. Called by: vw-core-api (`browser_fill`).
//! Invariants:
//! - [`Browser::fill_check`] runs no vault code and puts no value into a page; every refusal names the failed check.
//! - The value lives only in a zeroizing buffer here, is never logged, never put into an error or the action log, and
//!   the page script's answers never carry it (nor a page exception text).
//! - The page's answer is used only when it comes from the document the check was made for (`reading_allowed`).

use serde::Deserialize;
use serde_json::json;
use url::Url;
use zeroize::Zeroizing;

use crate::driver::{Browser, LOCAL_NETWORK, STOPPED};
use crate::model::{is_ref, parse_answer};
use crate::sessions::{Actor, Outcome};

/// The role of the field a login goes into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginRole {
    /// The password field (needs the user's grant).
    Password,
    /// The user-name field (the user name is not a secret: no grant).
    Username,
}

impl LoginRole {
    /// The wire word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Username => "username",
        }
    }
}

/// A field that passed every check before the vault is touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FillTarget {
    /// The ref.
    pub reference: String,
    /// Its role.
    pub role: LoginRole,
    /// The page's origin (`https://accounts.example.com`), the same in the observation and the page now.
    pub origin: String,
    /// The page's host (lower case).
    pub host: String,
    /// The field's role and name for the log and the dialog (page text, clipped).
    pub label: String,
}

/// Why a fill was refused: the check that failed (a stable word) and the reason the agent reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FillRefusal {
    /// `stopped`, `localNetwork`, `observation`, `currentObservation`, `credentialField`, `securePage`, `formTarget`,
    /// `formMethod`, `pageOrigin`, `frameOrigin`, `notActionable`, `page`, plus the core's own (`login`, `grant`, `vault`).
    pub check: &'static str,
    /// The reason.
    pub text: String,
}

fn refusal(check: &'static str, text: impl Into<String>) -> FillRefusal {
    FillRefusal {
        check,
        text: text.into(),
    }
}

#[derive(Deserialize)]
struct Answer {
    status: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    failed: Vec<String>,
    #[serde(default, rename = "submitsTo")]
    submits_to: Vec<String>,
}

/// `(origin, host)` of a page address, `None` when it has no tuple origin.
fn origin_of(address: &str) -> Option<(String, String)> {
    let url = Url::parse(address).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    Some((url.origin().ascii_serialization(), host))
}

/// ShellX's page-security rule: credentials only on https, or plain http on this computer.
fn secure(address: &str) -> bool {
    let Ok(url) = Url::parse(address) else {
        return false;
    };
    match url.scheme() {
        "https" => true,
        "http" => {
            matches!(
                url.host(),
                Some(url::Host::Ipv4(ip)) if ip.is_loopback()
            ) || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
                || url.host_str() == Some("localhost")
        }
        _ => false,
    }
}

/// Origins clipped and made safe to quote (page data: only URL-origin characters pass).
fn shown(origins: &[String]) -> String {
    origins
        .iter()
        .take(3)
        .map(|o| {
            o.chars()
                .filter(|c| c.is_ascii_alphanumeric() || ":/.-[]".contains(*c))
                .take(100)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl Browser {
    /// Every check before the vault is touched (module docs): the agent's Stop and the Lan rule, a current host-held
    /// observation that lists `reference` as a sign-in field, a secure page, a form that submits only to the page's
    /// own origin, then the same checks in the page now (no value in the script).
    ///
    /// # Errors
    /// A [`FillRefusal`] naming the failed check.
    pub async fn fill_check(
        &self,
        session_id: &str,
        reference: &str,
        by: Actor,
    ) -> Result<FillTarget, FillRefusal> {
        if by == Actor::Agent && self.sessions.stopped(session_id) {
            return Err(refusal("stopped", STOPPED));
        }
        if by == Actor::Agent && self.on_local_network(session_id) {
            return Err(refusal("localNetwork", LOCAL_NETWORK));
        }
        if !is_ref(reference) {
            return Err(refusal(
                "observation",
                "ref must be a ref from browser_read (e + 8 hex digits)",
            ));
        }
        let seen = self.sessions.seen(session_id).ok_or_else(|| {
            refusal(
                "observation",
                "there is no observation of this page yet: call browser_read first and use its refs",
            )
        })?;
        if seen.model.frames.iter().any(|f| f.id == reference) {
            return Err(refusal(
                "credentialField",
                format!(
                    "{reference} is a frame, not a sign-in field: fields inside frames from another site are never \
                     filled"
                ),
            ));
        }
        let item = seen
            .model
            .refs
            .iter()
            .find(|r| r.id == reference)
            .ok_or_else(|| {
                refusal(
                    "observation",
                    format!(
                        "ref {reference} is not listed in your latest browser_read: read again (find: 'password' \
                         lists the sign-in fields) and use a ref from it"
                    ),
                )
            })?;
        let (origin, host) = self.same_document(session_id, &seen)?;
        let role = match item.login.as_deref() {
            Some("password") => LoginRole::Password,
            Some("username") => LoginRole::Username,
            _ => {
                return Err(refusal(
                    "credentialField",
                    format!(
                        "{reference} is not a password field or the user-name field of a sign-in form: Vibe Work \
                         fills a vault login only there"
                    ),
                ));
            }
        };
        if !secure(&seen.model.url) {
            return Err(refusal(
                "securePage",
                "this page is not secure (http): a login is filled only on https pages (or on this computer)",
            ));
        }
        let targets = item.submits_to.clone().unwrap_or_default();
        if targets.iter().any(|t| t != &origin) {
            return Err(refusal(
                "formTarget",
                format!(
                    "the field's form sends to another site ({}), not to {origin}: not filled",
                    shown(&targets)
                ),
            ));
        }
        let target = FillTarget {
            reference: reference.to_owned(),
            role,
            label: format!(
                "{} '{}'",
                item.role,
                item.name.chars().take(60).collect::<String>()
            ),
            origin,
            host,
        };
        self.fill_page(session_id, &target, None).await?;
        Ok(target)
    }

    /// The observed page's `(origin, host)` when the mount still shows the observed document (same load, same
    /// origin): the current host-held observation.
    fn same_document(
        &self,
        session_id: &str,
        seen: &crate::sessions::Seen,
    ) -> Result<(String, String), FillRefusal> {
        let now = self.service.load_state(session_id);
        let same_document = self.service.page_mount(session_id).as_ref() == Some(&seen.mount)
            && seen.loads.is_some()
            && now.as_ref().map(|l| l.loads) == seen.loads;
        let (origin, host) = origin_of(&seen.model.url).ok_or_else(|| {
            refusal(
                "securePage",
                "this page has no web origin: logins are filled only on web pages",
            )
        })?;
        let mounted_origin = now
            .as_ref()
            .and_then(|l| l.url.as_deref())
            .and_then(origin_of)
            .map(|(o, _)| o);
        if !same_document || mounted_origin.as_deref() != Some(origin.as_str()) {
            return Err(refusal(
                "currentObservation",
                "the page changed since your latest browser_read (a new document or another site): call \
                 browser_read again",
            ));
        }
        Ok((origin, host))
    }

    /// Writes `value` into the field of `target` (the page script repeats every check in the same call). Records the
    /// fill in the session's action log (never the value).
    ///
    /// # Errors
    /// A [`FillRefusal`] naming the failed check (the page changed since [`Browser::fill_check`]).
    pub async fn fill_write(
        &self,
        session_id: &str,
        target: &FillTarget,
        value: &str,
        by: Actor,
    ) -> Result<(), FillRefusal> {
        if by == Actor::Agent && self.sessions.stopped(session_id) {
            return Err(refusal("stopped", STOPPED));
        }
        let _flight = self.sessions.begin(session_id, by);
        if target.role == LoginRole::Password {
            // Before the write: no screenshot of this document from now on, so a
            // page that shows the password in clear (a swapped "show password" field) is never pictured.
            self.service.block_shots_on_this_document(session_id);
        }
        let result = self.fill_page(session_id, target, Some(value)).await;
        self.sessions.end(
            session_id,
            by,
            "fill",
            &target.label,
            if result.is_ok() {
                Outcome::Done
            } else {
                Outcome::Refused
            },
            match &result {
                Ok(()) => "filled from the vault (the value is not shown)",
                Err(e) => e.text.as_str(),
            },
        );
        result
    }

    /// Records a refused fill in the session's action log (the core's refusals: login, grant, vault).
    pub fn fill_refused(&self, session_id: &str, by: Actor, reference: &str, reason: &str) {
        self.sessions
            .end(session_id, by, "fill", reference, Outcome::Refused, reason);
    }

    /// Runs `js/fill.js` once (no retry: a write is never repeated into a document that replaced this one).
    async fn fill_page(
        &self,
        session_id: &str,
        target: &FillTarget,
        value: Option<&str>,
    ) -> Result<(), FillRefusal> {
        let page = self
            .page(session_id)
            .await
            .map_err(|e| refusal("page", e))?;
        let mut request = json!({
            "page": page,
            "ref": target.reference,
            "role": target.role.as_str(),
            "origin": target.origin,
        });
        match value {
            Some(v) => request["value"] = json!(v),
            None => request["check"] = json!(true),
        }
        let script = Zeroizing::new(crate::scripts::fill(&request));
        if let Some(v) = request.get_mut("value") {
            // The request's copy of the value goes now; the script's copy when `script` drops.
            *v = json!("");
        }
        let raw = self
            .service
            .evaluate(session_id, script.to_string())
            .await
            .map_err(|_| refusal("page", "the page did not answer the fill"))?;
        drop(script);
        let answer: Answer = parse_answer(&raw).map_err(|e| refusal("page", e))?;
        self.service
            .reading_allowed(session_id, &page, &answer.url)
            .map_err(|e| refusal("currentObservation", e))?;
        let reference = &target.reference;
        match answer.status.as_str() {
            "ok" if value.is_none() => Ok(()),
            "filled" if value.is_some() => Ok(()),
            "stale" | "ambiguous" => Err(refusal(
                "observation",
                format!("ref {reference} is not on the page any more: call browser_read again"),
            )),
            "originChanged" => Err(refusal(
                "pageOrigin",
                format!(
                    "the page is no longer on {} (it navigated): call browser_read again",
                    target.origin
                ),
            )),
            "frameOrigin" => Err(refusal(
                "frameOrigin",
                "the field is inside a frame from another site: not filled",
            )),
            "notCredential" => Err(refusal(
                "credentialField",
                format!(
                    "{reference} is no longer a {} field of a sign-in form: not filled",
                    target.role.as_str()
                ),
            )),
            "formMethod" => Err(refusal(
                "formMethod",
                "the field's form sends with GET (the password would end up in the page address): not filled",
            )),
            "formOrigin" => Err(refusal(
                "formTarget",
                format!(
                    "the field's form now sends to another site ({}): not filled",
                    shown(&answer.submits_to)
                ),
            )),
            "notActionable" => Err(refusal(
                "notActionable",
                format!(
                    "the field cannot be filled now ({}): call browser_read again",
                    answer
                        .failed
                        .iter()
                        .map(|f| f
                            .chars()
                            .filter(char::is_ascii_alphabetic)
                            .take(20)
                            .collect::<String>())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
            "cancelled" => Err(refusal("page", "the page cancelled the input: not filled")),
            _ => Err(refusal("page", "the page did not accept the fill")),
        }
    }
}
