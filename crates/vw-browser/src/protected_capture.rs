//! Visual captures never show a protected field's value. Before the engine renders pixels or a
//! print for the agent (a `browser_read` screenshot, `capture.screenshot`, an element crop, a GIF frame, a full-page
//! PNG or PDF, the App preview's `preview_screenshot`), every frame of the page is checked with the shared
//! privateField rule (`js/protected_shown.js`), and the capture is refused while a protected field shows a readable
//! value.
//! It runs again after the pixels are taken, so a value typed during the capture refuses it too. A frame that cannot
//! be checked refuses the capture. A closed shadow root cannot be entered from this script world: the page's
//! keeper (vw-preview `js/closed_roots.js`) answers for the roots `attachShadow` created, and a filled field in one
//! refuses the capture; a document without the keeper refuses it too. A closed root the HTML parser made (declarative
//! shadow DOM) is read by an engine whose world enters closed roots (`WebView2` through the protocol, macOS 27);
//! elsewhere the engine's check of the documents' sources (`PreviewEngine::declarative_closed_roots`) refuses a page
//! that may hold one, and the keeper marks one made by script.
use crate::{Browser, scripts};

/// The refusal of a visual capture while the page shows a protected value.
#[must_use]
pub fn protected_shown() -> String {
    "Not run: this page shows a protected field value. Capture was withheld so the value stays private.".into()
}

/// A recording whose first frame was withheld because the page shows a protected value answers with the
/// approved refusal alone: it is final, so it is not wrapped in a first-frame failure and no other capture
/// route (another recording kind, "make the tab visible") is offered. `None` for every other failure.
#[must_use]
pub fn first_frame_refusal(reason: &str) -> Option<String> {
    (reason.trim() == protected_shown()).then(protected_shown)
}

impl Browser {
    /// Counts protected fields in the current page before a raw script or data read.
    pub(crate) async fn protected_field_count(&self, session: &str) -> Result<u64, String> {
        let mount = self
            .service
            .page_mount(session)
            .ok_or_else(protected_read_refused)?;
        let script = scripts::protected_count();
        let (answers, urls) = match self
            .service
            .evaluate_frames_on_mount(&mount, None, script.clone(), None)
            .await
        {
            Ok(Some(answers)) => split_answers(answers),
            Ok(None) => (
                vec![
                    self.service
                        .evaluate_on_mount(&mount, script)
                        .await
                        .map_err(|_| protected_read_refused())?,
                ],
                self.page_url(session),
            ),
            Err(_) => return Err(protected_read_refused()),
        };
        let (mut named, mut answered, mut count) = (1_u64, 0_u64, 0_u64);
        for value in &answers {
            let Some(row) = counted(value) else {
                return Err(protected_read_refused());
            };
            if row.entered {
                continue;
            }
            named = named.saturating_add(row.frames);
            answered += 1;
            count = count.saturating_add(row.count);
            if count > 10000 {
                return Err(protected_read_refused());
            }
        }
        if answered < named {
            return Err(protected_read_refused());
        }
        // A closed root the parser made, where the engine's world cannot enter it: counted as one protected field.
        if count == 0 && self.declarative(&mount, urls).await {
            return Ok(1);
        }
        Ok(count)
    }

    /// Whether the engine reports a document of the page whose source may hold a closed declarative shadow root it
    /// cannot enter; an engine failure counts as one.
    async fn declarative(&self, mount: &vw_preview::MountId, urls: Vec<String>) -> bool {
        self.service
            .declarative_closed_roots_on_mount(mount, urls)
            .await
            .unwrap_or(true)
    }

    /// The session page's address, for an engine without frame evaluation.
    fn page_url(&self, session: &str) -> Vec<String> {
        self.service
            .view(session)
            .and_then(|view| view.url)
            .into_iter()
            .collect()
    }
    /// Admits a visual capture of `session`'s page for the agent. Every agent-bound capture of a mount runs it
    /// before and after taking the pixels, the App preview's `preview_screenshot` included.
    ///
    /// # Errors
    /// [`protected_shown`] while a frame shows a protected value or cannot be checked.
    pub async fn admit_capture(&self, session: &str) -> Result<(), String> {
        let mount = self
            .service
            .page_mount(session)
            .ok_or_else(protected_shown)?;
        let script = scripts::protected_shown();
        let (answers, urls) = match self
            .service
            .evaluate_frames_on_mount(&mount, None, script.clone(), None)
            .await
        {
            Ok(Some(answers)) => split_answers(answers),
            // An engine without frame evaluation checks the main document.
            Ok(None) => (
                vec![
                    self.service
                        .evaluate_on_mount(&mount, script)
                        .await
                        .map_err(|_| protected_shown())?,
                ],
                self.page_url(session),
            ),
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
        if answered < named || self.declarative(&mount, urls).await {
            return Err(protected_shown());
        }
        Ok(())
    }
}

/// Each frame's answer value and document address.
fn split_answers(answers: Vec<vw_preview::FrameAnswer>) -> (Vec<String>, Vec<String>) {
    answers
        .into_iter()
        .map(|answer| (answer.value, answer.url))
        .unzip()
}

pub(crate) fn protected_read_refused() -> String {
    "Not run: this page has a card number, PIN or password field, so page scripts and raw data reads are off here. Use page.read, page.type, page.click, or auth.fill with a vault login. Protected field values were withheld from this result.".into()
}

struct Counted {
    count: u64,
    frames: u64,
    entered: bool,
}
fn counted(value: &str) -> Option<Counted> {
    let mut parsed: serde_json::Value = serde_json::from_str(value).ok()?;
    if let Some(inner) = parsed.as_str() {
        parsed = serde_json::from_str(inner).ok()?;
    }
    let count = parsed["count"].as_u64().filter(|n| *n <= 10000)?;
    let frames = parsed["frames"].as_u64().filter(|n| *n <= 128)?;
    let entered = parsed["entered"].as_bool()?;
    Some(Counted {
        count,
        frames,
        entered,
    })
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

#[cfg(test)]
mod first_frame_tests {
    use super::*;

    #[test]
    fn a_withheld_first_frame_is_the_bare_approved_refusal() {
        assert_eq!(
            first_frame_refusal(&protected_shown()),
            Some(protected_shown())
        );
        assert_eq!(first_frame_refusal("the tab is hidden"), None);
    }
}
