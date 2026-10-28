//! Native confirmation: the [`Confirmer`] trait, the single-use token vault and its clock.
//!
//! Depends on: canonical, error, ids. Called by: gate (mint on approval, redeem on call).
//! Invariants:
//! - A token is minted only after the confirmer reported `Approved` for one exact method and params.
//! - A token redeems at most once, only for the same method and the same params (compared as
//!   canonical JSON text, not a lossy hash), and only before it expires.
//! - A refused redemption (wrong params, wrong method) does not consume the token.
//! - The vault is bounded; the oldest entry is dropped first.
//!
//! The real [`Confirmer`] arrives with the UI blocks: it must show the request in an OS-native
//! dialog owned by the core (Tauri dialog), never in webview HTML, because the webview is the
//! untrusted side. Tests use a scripted fake.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::canonical::canonical;
use crate::error::{GateError, TokenFault};
use crate::ids::random_hex;

/// Who asked for a confirmation: a structured field, so the dialog can always
/// lead with it; never derived from the wording of the summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requester {
    /// A click in the app's own window.
    Window,
    /// An external client through the Debug API. It acts as the user: the core's confirmer
    /// answers its requests without a dialog, except the vault's dialogs (`vault_scope::DIALOG_METHODS`), which only
    /// the user answers. Only the core sets it, from the command's origin.
    DebugApi,
    /// The agent of a session (a command it wants to run).
    Agent,
    /// Vibe Work itself, finishing a step the user chose earlier: deleting a temporary session at quit or at the
    /// next start (the user's consent is the "Temporary chat" switch, whose text says so). The core's
    /// confirmer answers it without a dialog. Only the core sets it, never from a client's request.
    Core,
}

/// What the user is asked to approve, shown by the native dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmationRequest {
    /// The method to be called.
    pub method: String,
    /// Complete plain-language consent text shown in the native dialog. Include every fact the user needs to
    /// decide, including the full command when one will run; the host never renders `method` or `params`.
    pub summary: String,
    /// The exact params used to bind the confirmation token; these are never rendered in the dialog.
    pub params: Value,
    /// Who asked; the dialog leads with it when it is not the window.
    pub requester: Requester,
    /// A third answer to offer (the secret broker's "Always in this project"): its button label. `None`
    /// (every other request) = the usual two buttons. A confirmer that cannot show three buttons shows two and
    /// never answers [`Confirmation::ApprovedAlways`].
    pub always: Option<String>,
}

/// The user's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    /// The user approved this exact request.
    Approved,
    /// The user declined, closed the dialog, or no dialog could be shown.
    Declined,
    /// The user chose the request's [`ConfirmationRequest::always`] answer (only for a request that offers it; the
    /// core's confirmer turns it into [`Confirmation::Approved`] for any other request).
    ApprovedAlways,
}

impl Confirmation {
    /// Shared agent-facing sentence for a declined permission.
    #[must_use]
    pub fn agent_text(self, what: &str) -> String {
        match self {
            Self::Declined => vw_ask::agent_text(
                &vw_ask::Outcome::Refused {
                    by: vw_ask::AnsweredBy::Person,
                    note: None,
                },
                what,
            ),
            _ => String::new(),
        }
    }

    /// True for both approvals (this request is approved either way).
    #[must_use]
    pub fn approved(self) -> bool {
        matches!(self, Self::Approved | Self::ApprovedAlways)
    }
}

/// Shows a request in an OS-native dialog and reports the user's decision.
pub trait Confirmer: Send + Sync {
    /// Asks the user. Implementations must fail closed: return [`Confirmation::Declined`] when the
    /// dialog cannot be shown.
    fn confirm(&self, request: &ConfirmationRequest) -> impl Future<Output = Confirmation> + Send;
}

/// Time source, replaceable in tests.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> Instant;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// An opaque single-use proof that the user approved one exact request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationToken(String);

impl ConfirmationToken {
    /// The token text (what travels to the UI and back).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Rebuilds a token from the text the UI sent back. Holding the text proves nothing: the vault
    /// still has to know it.
    #[must_use]
    pub fn from_wire(text: String) -> Self {
        Self(text)
    }
}

struct Entry {
    method: String,
    params: String,
    expires: Instant,
    spent: bool,
}

/// Issued tokens, bounded and expiring.
pub(crate) struct TokenVault {
    entries: HashMap<String, Entry>,
    order: VecDeque<String>,
    ttl: Duration,
    max: usize,
}

impl TokenVault {
    pub(crate) fn new(ttl: Duration, max: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            ttl,
            max: max.max(1),
        }
    }

    /// Issues a token for `method` + `params`, valid for the vault's lifetime from `now`.
    pub(crate) fn mint(
        &mut self,
        method: &str,
        params: &Value,
        now: Instant,
    ) -> Result<ConfirmationToken, GateError> {
        self.sweep(now);
        while self.entries.len() >= self.max {
            match self.order.pop_front() {
                Some(oldest) => {
                    self.entries.remove(&oldest);
                }
                None => break,
            }
        }
        let text = random_hex(16)?;
        self.entries.insert(
            text.clone(),
            Entry {
                method: method.to_owned(),
                params: canonical(params),
                expires: now + self.ttl,
                spent: false,
            },
        );
        self.order.push_back(text.clone());
        Ok(ConfirmationToken(text))
    }

    /// What [`TokenVault::redeem`] would answer, without spending the token.
    pub(crate) fn check(
        &self,
        method: &str,
        params: &Value,
        token: &ConfirmationToken,
        now: Instant,
    ) -> Result<(), TokenFault> {
        let Some(entry) = self.entries.get(token.as_str()) else {
            return Err(TokenFault::Unknown);
        };
        if entry.spent {
            return Err(TokenFault::Reused);
        }
        if now >= entry.expires {
            return Err(TokenFault::Expired);
        }
        if entry.method != method || entry.params != canonical(params) {
            return Err(TokenFault::Mismatch);
        }
        Ok(())
    }

    /// Checks `token` for `method` + `params` and, only when everything matches, spends it.
    pub(crate) fn redeem(
        &mut self,
        method: &str,
        params: &Value,
        token: &ConfirmationToken,
        now: Instant,
    ) -> Result<(), TokenFault> {
        let Some(entry) = self.entries.get_mut(token.as_str()) else {
            return Err(TokenFault::Unknown);
        };
        if entry.spent {
            return Err(TokenFault::Reused);
        }
        if now >= entry.expires {
            return Err(TokenFault::Expired);
        }
        if entry.method != method || entry.params != canonical(params) {
            return Err(TokenFault::Mismatch);
        }
        entry.spent = true;
        Ok(())
    }

    /// Drops entries whose lifetime is over (spent ones included: a reused token past its
    /// lifetime is then simply unknown).
    fn sweep(&mut self, now: Instant) {
        self.entries.retain(|_, e| now < e.expires);
        let entries = &self.entries;
        self.order.retain(|t| entries.contains_key(t));
    }
}
