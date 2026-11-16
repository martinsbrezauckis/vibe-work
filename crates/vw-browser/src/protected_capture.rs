//! Visual captures below Full access never show a protected field's value. Before the engine renders pixels or a
//! print for the agent (a `browser_read` screenshot, `capture.screenshot`, an element crop, a GIF frame, a full-page
//! PNG or PDF, the App preview's `preview_screenshot`), every frame of the page is checked with the shared
//! privateField rule (`js/protected_shown.js`), and the capture is refused while a protected field shows a readable
//! value. Full access (the core's [`crate::delegation::DelegationAuthorizer::protected_values_delegated`]) delegates
//! the page's data.
//! It runs again after the pixels are taken, so a value typed during the capture refuses it too. A frame that cannot
//! be checked refuses the capture. A closed shadow root cannot be entered from this script world: the page's
//! keeper (vw-preview `js/closed_roots.js`) answers for the roots `attachShadow` created, and a filled field in one
//! refuses the capture; a document without the keeper refuses it too.
use crate::{Browser, scripts};

/// The refusal of a visual capture below Full access while the page shows a protected value. The mode name comes
/// from the vocabulary, like every other mode refusal.
#[must_use]
pub fn protected_shown() -> String {
    format!(
        "Not run: this page shows a password, PIN or card value. Capturing it needs {}.",
        vw_ask::chrome::Tier::FullAccess.label()
    )
}

impl Browser {
    /// Admits a visual capture of `session`'s page for the agent. Every agent-bound capture of a mount runs it
    /// before and after taking the pixels, the App preview's `preview_screenshot` included.
    ///
    /// # Errors
    /// [`protected_shown`] while a frame shows a protected value or cannot be checked.
    pub async fn admit_capture(&self, session: &str) -> Result<(), String> {
        let delegated = self.delegation_authorizer.read().is_ok_and(|slot| {
            slot.as_ref()
                .is_some_and(|authorizer| authorizer.protected_values_delegated(session))
        });
        if delegated {
            return Ok(());
        }
        let mount = self
            .service
            .page_mount(session)
            .ok_or_else(protected_shown)?;
        let script = scripts::protected_shown();
        let answers = match self
            .service
            .evaluate_frames_on_mount(&mount, None, script.clone(), None)
            .await
        {
            Ok(Some(answers)) => answers.into_iter().map(|answer| answer.value).collect(),
            // An engine without frame evaluation checks the main document.
            Ok(None) => vec![
                self.service
                    .evaluate_on_mount(&mount, script)
                    .await
                    .map_err(|_| protected_shown())?,
            ],
            Err(_) => return Err(protected_shown()),
        };
        // Every frame must answer "nothing shown". Each frame its parent cannot enter is named once (by the nearest
        // document that walked its parent); one of them the engine did not answer for refuses too.
        let (mut named, mut answered) = (1_u64, 0_u64);
        for value in &answers {
            match clear(value) {
                Some(Clear::Entered) => {}
                Some(Clear::Own(frames)) => {
                    named = named.saturating_add(frames);
                    answered += 1;
                }
                None => return Err(protected_shown()),
            }
        }
        if answered < named {
            return Err(protected_shown());
        }
        Ok(())
    }
}

/// A frame that answered "no protected value shown".
enum Clear {
    /// Its parent document entered it (and so already looked at it).
    Entered,
    /// Its parent could not enter it; it names this many child frames it cannot enter itself.
    Own(u64),
}

/// One frame's answer when it is exactly "no protected value shown" (it may arrive JSON-encoded once more); `None`
/// for anything else.
fn clear(value: &str) -> Option<Clear> {
    let mut parsed: serde_json::Value = serde_json::from_str(value).unwrap_or_default();
    if let Some(inner) = parsed.as_str() {
        parsed = serde_json::from_str(inner).unwrap_or_default();
    }
    if parsed["shown"] != false {
        return None;
    }
    Some(if parsed["entered"] == true {
        Clear::Entered
    } else {
        Clear::Own(parsed["frames"].as_u64().unwrap_or(u64::MAX))
    })
}
