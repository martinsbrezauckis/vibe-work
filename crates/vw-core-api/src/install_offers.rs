//! One-click installs from the window: a visible Install / Update click in Vibe Work's
//! own window is the consent, with no extra native dialog, like the Vibe setup install (`vibe_install.rs`).
//!
//! The pattern is an offer the window shows first, then a click that confirms exactly that offer:
//! 1. `offer…` builds an [`ToolsInstallOffer`] from what Vibe Work itself read (name, publisher or source, licence, what
//!    runs) and keeps it under a fresh random id, bound to the exact action (`key`: kind + target + folder);
//! 2. the install or update naming that `offerId` takes the offer (single use, whatever the result), and only then
//!    skips the native dialog.
//!
//! A missing offer keeps the native dialog, so a raw call from the window cannot install
//! something the user was not shown. A foreign (another target or folder), reused or unknown id is refused with
//! `serverCode` [`STALE_OFFER`]; an expired one with its own code [`OFFER_EXPIRED`], so the window can say "ran out"
//! without reading the message. Only the window's origin skips the dialog this way; the Debug API keeps acting as the user
//! through its own confirmer, unchanged. The agent has no route to these commands; anything an agent starts keeps
//! its ask. Turning a plugin on (`vw/plugins/turnOn`) and deletes keep their confirmations: no offer exists
//! for them.
//!
//! Depends on: `getrandom` (ids), the ask clock (expiry). Called by: `skills` (import, setLatest), `market`
//! (marketplace install and update).
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, ErrorCode};

/// How long an offer stays valid after the window got it.
const OFFER_TTL_MS: u64 = 10 * 60 * 1000;
/// Most offers kept at once; the oldest goes first. Expired offers are kept until then (or until taken), so a click on
/// one is answered [`OFFER_EXPIRED`] even after newer offers were made.
const MAX_OFFERS: usize = 64;

/// `serverCode` of a refused offer that was used, unknown, made for another action or changed since it was shown.
pub(crate) const STALE_OFFER: &str = "staleOffer";
/// `serverCode` of a refused offer whose time ran out (`OFFER_TTL_MS`).
pub(crate) const OFFER_EXPIRED: &str = "offerExpired";

/// What the window shows before the click; the click confirms this exact offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ToolsInstallOffer {
    /// Single-use id the click sends back.
    pub offer_id: String,
    /// `skill`, `skillUpdate` or `marketplace`.
    pub kind: String,
    /// The name shown.
    pub name: String,
    /// Who publishes it, when known.
    pub publisher: Option<String>,
    /// Where it comes from (the skill registry and its id, or the reviewed Tools catalog).
    pub source: String,
    /// Its licence, when the source states one.
    pub licence: Option<String>,
    /// What it adds or runs, in plain words.
    pub runs: String,
    /// The version installed by the click, when known.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub version: Option<i64>,
    /// Unix milliseconds after which the click is refused (ask the window for a fresh offer).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub expires_at: u64,
}

/// What an offer is for, apart from what it shows: the exact action it confirms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OfferKey(String);

impl OfferKey {
    /// The key of one action: its kind, target and the folder it runs in (empty when global).
    pub(crate) fn new(kind: &str, target: &str, folder: &str) -> Self {
        Self(format!("{kind}\u{0}{target}\u{0}{folder}"))
    }
}

struct Kept {
    key: OfferKey,
    expires_at: u64,
    /// The version the offer showed: an install pins it, an update is refused when the source moved on.
    version: Option<i64>,
}

/// The run's open install offers (one per core).
#[derive(Default)]
pub(crate) struct Offers {
    state: Mutex<BTreeMap<String, Kept>>,
}

fn refused(code: &str, message: &str) -> CoreError {
    let mut error = CoreError::new(ErrorCode::InvalidParams, message);
    error.server_code = Some(code.to_owned());
    error
}

fn stale(message: &str) -> CoreError {
    refused(STALE_OFFER, message)
}

fn random_id() -> Option<String> {
    let mut bytes = [0_u8; 16];
    // No predictable fallback: an offer id is the only thing that tells a shown offer from a raw call.
    getrandom::fill(&mut bytes).ok()?;
    let mut id = String::from("offer-");
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(id, "{byte:02x}");
    }
    Some(id)
}

impl Offers {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Kept>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps a new offer for `key` and returns it with its id and expiry filled in.
    ///
    /// # Errors
    /// `internal` when no random id is available (no offer is made then; the dialog path still works).
    pub(crate) fn make(
        &self,
        key: OfferKey,
        mut offer: ToolsInstallOffer,
        now: u64,
    ) -> Result<ToolsInstallOffer, CoreError> {
        let id = random_id().ok_or_else(|| {
            CoreError::new(ErrorCode::Internal, "No install offer could be prepared.")
        })?;
        let mut state = self.lock();
        // Bounded by the count only (the oldest, so expired ones, go first): an expired offer stays until then, so its
        // click is told `offerExpired`, not "unknown".
        while state.len() >= MAX_OFFERS {
            let oldest = state
                .iter()
                .min_by_key(|(_, k)| k.expires_at)
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => state.remove(&id),
                None => break,
            };
        }
        offer.offer_id.clone_from(&id);
        offer.expires_at = now.saturating_add(OFFER_TTL_MS);
        state.insert(
            id,
            Kept {
                key,
                expires_at: offer.expires_at,
                version: offer.version,
            },
        );
        Ok(offer)
    }

    /// Takes the offer `id` for the action `key`: it is gone afterwards whether or not it matched. Returns the
    /// version the offer showed (the caller installs exactly that one, or refuses with [`changed`]).
    ///
    /// # Errors
    /// `invalidParams` with `serverCode` [`STALE_OFFER`] for an unknown or used id or an offer made for another
    /// action; [`OFFER_EXPIRED`] for an expired offer.
    pub(crate) fn take(
        &self,
        id: &str,
        key: &OfferKey,
        now: u64,
    ) -> Result<Option<i64>, CoreError> {
        let kept = self.lock().remove(id).ok_or_else(|| {
            stale("This install offer was already used or is unknown. Look at the offer again.")
        })?;
        if kept.expires_at <= now {
            return Err(refused(
                OFFER_EXPIRED,
                "This install offer has expired. Look at the offer again.",
            ));
        }
        if &kept.key != key {
            return Err(stale("This install offer is for something else."));
        }
        Ok(kept.version)
    }
}

/// The [`STALE_OFFER`] refusal for an offer whose source changed after the window showed it (a newer version).
pub(crate) fn changed() -> CoreError {
    stale("This changed since the offer was shown. Look at the offer again.")
}
