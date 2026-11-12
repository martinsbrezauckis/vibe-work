//! What the engine refused on the person's behalf during an agent act, as receipt notes the agent reads: a file
//! picker (never shown) and full screen. Source: the engine's counts (`PreviewService::act_refusal_mark`), which
//! every system reports from the view itself (Linux and macOS 11-12 before the request goes through, macOS 13+ and
//! Windows when the view enters full screen, which it then leaves). A request the browser refused by itself (no
//! user activation left) never reached the engine and gets no note; the page saw its own error.
//!
//! Depends on: vw-preview. Called by: `driver::perform` (marks before the act, notes after the receipt's settle).

use serde_json::{Value, json};
use vw_preview::ActRefusal;

use crate::driver::Browser;

const KINDS: [ActRefusal; 2] = [ActRefusal::FilePicker, ActRefusal::FullScreen];

/// The engine's counts before an act.
pub(crate) struct Marks([u64; 2]);

impl Browser {
    /// The counts this act's notes compare with: those last reported for the session, so a refusal during an act
    /// that ended without a receipt (a failed or unconfirmed act) is reported on the next receipt.
    pub(crate) fn act_marks(&self, session_id: &str) -> Marks {
        let reported = self
            .act_reported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session_id)
            .copied();
        Marks(reported.unwrap_or_else(|| self.counts(session_id)))
    }

    fn counts(&self, session_id: &str) -> [u64; 2] {
        KINDS.map(|kind| self.service.act_refusal_mark(session_id, kind))
    }

    /// Adds the notes for what was refused during this act (after its settle wait, when the engine has seen it).
    pub(crate) fn act_notes(
        &self,
        session_id: &str,
        reference: Option<&str>,
        before: &Marks,
        receipt: &mut Value,
    ) {
        let now = self.counts(session_id);
        self.act_reported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session_id.to_owned(), now);
        let grew = |kind: ActRefusal| {
            let index = KINDS.iter().position(|k| *k == kind).unwrap_or(0);
            now[index] > before.0[index]
        };
        if grew(ActRefusal::FilePicker) {
            receipt["filePicker"] = json!(crate::upload::picker_note(reference));
        }
        if grew(ActRefusal::FullScreen) {
            receipt["fullScreen"] = json!(
                "The page asked for full screen; it was refused, because agent acts may not take over the user's screen. Continue in the Browser pane."
            );
        }
    }
}
