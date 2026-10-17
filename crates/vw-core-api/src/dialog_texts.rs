//! Dialog sentences the core writes for one exact gated call: the gate asks the native
//! confirmation with its own generic sentence, but a worktree removal must name the files it discards, which
//! are not in the call's params. Before asking, the core registers the sentence for that exact method and
//! params; the confirmer (`confirmer.rs`) takes it when the gate's request arrives.
//!
//! Depends on: `serde_json`. Called by: `worktrees_remove` and `network` (set), `confirmer` (takes).
//! Invariants:
//! - A sentence is used once, only for the identical method and params it was registered for; anything else
//!   gets the gate's sentence (or the confirmer's own per-method text).
//! - Every untrusted value inside a sentence was sanitized by its author (`vw_gate::dialog_value` /
//!   `dialog_line`); the requester is the dialog's structured field, never part of this text.
//! - At most [`MAX_PENDING`] sentences wait; older ones are dropped (a dialog that never came).

use std::sync::{Mutex, PoisonError};

use serde_json::Value;

/// Sentences kept waiting at most.
const MAX_PENDING: usize = 16;

/// The registry.
#[derive(Default)]
pub(crate) struct DialogTexts {
    pending: Mutex<Vec<(String, Value, DialogText)>>,
}

pub(crate) struct DialogText {
    text: String,
    suffix: bool,
}

impl DialogText {
    pub(crate) fn apply(self, summary: &mut String) {
        if self.suffix {
            summary.push_str(&self.text);
        } else {
            *summary = self.text;
        }
    }
}

impl DialogTexts {
    /// Registers `summary` for the next confirmation of exactly `method` with `params`.
    pub(crate) fn set(&self, method: &str, params: &Value, summary: String) {
        self.insert(method, params, summary, false);
    }

    /// Appends prepared details to the gate's existing sentence for exactly this confirmation.
    pub(crate) fn set_suffix(&self, method: &str, params: &Value, suffix: String) {
        self.insert(method, params, suffix, true);
    }

    fn insert(&self, method: &str, params: &Value, text: String, suffix: bool) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.retain(|(m, p, _)| !(m == method && p == params));
        if pending.len() >= MAX_PENDING {
            pending.remove(0);
        }
        pending.push((
            method.to_owned(),
            params.clone(),
            DialogText { text, suffix },
        ));
    }

    /// Takes the sentence registered for exactly `method` with `params`.
    pub(crate) fn take(&self, method: &str, params: &Value) -> Option<DialogText> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let at = pending
            .iter()
            .position(|(m, p, _)| m == method && p == params)?;
        Some(pending.remove(at).2)
    }
}
