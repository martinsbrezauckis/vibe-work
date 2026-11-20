//! Runs one capture: the checks (Stop and live page identity), the folder, each step, the
//! `capture.json` record, the action log entry, and the compact answer (Vibe Work's words first; page words under
//! `untrustedPageContent` with the prompt guard's verdict; within the answer budget).
//!
//! Depends on: the capture modules, `driver`, `sessions`, `guard`, `observe`. Called by: vw-core-api.
//! Invariant: every step is bounded; optional steps can be skipped while others run. The required manifest must
//! commit before a capture succeeds or its Before state is published or consumed.

use std::time::{Duration, SystemTime};

use serde_json::{Map, Value, json};

use super::design::safe_name;
use super::files::{Folder, slug, stamp};
use super::{Before, PageFormat, Request, Source, Target, What, shot};
use crate::driver::{ActError, Browser, STOPPED, failed, refused};
use crate::sessions::{Actor, Outcome};

#[path = "run_answer.rs"]
mod answer;
#[path = "run_record.rs"]
mod record;
#[path = "run_steps.rs"]
mod steps;
use answer::answer;
use record::{before_record, save_record};

/// The answer's first key (`Outcome`): Vibe Work's own words.
pub const OUTCOME_KEY: &str = crate::driver::HEADLINE_KEY;
/// A whole capture's limit (under the agent tools' 50 s call limit).
const CAPTURE_LIMIT: Duration = Duration::from_secs(46);
/// Caps per file.
const TEXT_CAP: u64 = 16 * 1024 * 1024;
const HTML_CAP: u64 = 48 * 1024 * 1024;
const PNG_CAP: u64 = 40 * 1024 * 1024;
const SMALL_CAP: u64 = 4 * 1024 * 1024;
// Forty media filenames plus the fixed capture files need more room than a page-read receipt.
const ANSWER_BUDGET: usize = 8_000;

/// The smallest view (CSS px of the page's layout viewport) a layout-dependent capture runs in. A browser
/// that is not on screen is parked at the size it would have when shown (or 1280 x 800), so a background session
/// captures like a shown one; this check stays for an engine that cannot do that (a webview stuck at its smallest
/// size would describe a tiny page, not the site). See [`check_view`].
const MIN_VIEW: (u32, u32) = (200, 120);

/// Refuses a layout-dependent capture of a page whose view has no usable size. `view` is the layout viewport from the
/// engine's native frame and the zoom it was placed with (`PreviewService::layout_viewport`), never pixels: pixel
/// screenshots are kept to a preview the user sees, and a background session's page is parked at the size it would have
/// when shown, so a hidden page passes here like a shown one (a check that took a screenshot refused every page not
/// on screen). Not the page's own numbers either: a page can redefine `innerWidth` (the
/// capture fixture's /liar reports 1 x 1).
fn check_view(view: Option<(f64, f64)>, request: &Request) -> Result<(), ActError> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (width, height) = view.map_or((0, 0), |(w, h)| {
        (w.max(0.0).floor() as u32, h.max(0.0).floor() as u32)
    });
    if width < MIN_VIEW.0 || height < MIN_VIEW.1 {
        return Err(refused(format!(
            "this session's browser has no usable size ({width} x {height} px), so a {} capture would \
             measure a tiny page: ask the user to show this session's Browser tab, then capture again (media \
             and the page's text formats work without it)",
            request.what.name()
        )));
    }
    Ok(())
}

/// True when the request's output includes a picture of the page (a PNG, the compare shots, the bundle's page.png).
fn takes_pixels(request: &Request) -> bool {
    match request.what {
        What::Before | What::After | What::Bundle => true,
        What::Page => request.formats.iter().any(|f| matches!(f, PageFormat::Png)),
        What::Design | What::Structure | What::Media => false,
    }
}

/// True when the request measures layout (sizes, boxes, pictures), so the page must have a real size.
fn needs_layout(request: &Request) -> bool {
    match request.what {
        What::Media => false,
        What::Page => request
            .formats
            .iter()
            .any(|f| matches!(f, PageFormat::Png | PageFormat::Pdf)),
        _ => true,
    }
}

fn folder_name_source(title: &str, url: &str, host: &str) -> String {
    // Filter before lowercasing/truncation can disguise a title key in the generated folder name.
    let mut filtered = json!({"title": title, "url": url});
    crate::output::filter_agent_output(&mut filtered);
    if filtered["title"] != title
        || vw_secrets::scrub(title.as_bytes()).is_some()
        || slug(title) == "page"
    {
        host.to_owned()
    } else {
        title.to_owned()
    }
}

/// What one capture produced so far.
struct Run {
    folder: Folder,
    source: Source,
    summary: Map<String, Value>,
    skipped: Vec<Value>,
    /// Page words for the prompt guard and the untrusted block: (channel, text).
    page_words: Vec<(String, String)>,
    fonts: Vec<String>,
    /// Published only after its manifest commits.
    before: Option<Before>,
}

impl Run {
    fn skip(&mut self, step: &str, reason: &str) {
        self.skipped
            .push(json!({"step": step, "reason": reason.chars().take(300).collect::<String>()}));
    }

    fn publish(&mut self, captures: &super::Captures, session_id: &str, what: What) {
        if let Some(before) = self.before.take() {
            captures.set_before(session_id, before);
        } else if what == What::After {
            captures.forget(session_id);
        }
    }
}

impl Browser {
    /// `browser_capture` / the Capture menu: one capture by `by`, answered compactly. The action log records it.
    ///
    /// # Errors
    /// The refusal or failure (`ActError`): Stop, nothing open, no folder.
    pub async fn capture(
        &self,
        session_id: &str,
        request: Request,
        by: Actor,
        target: Target<'_>,
    ) -> Result<Value, ActError> {
        let _capture = self.captures().begin(session_id).map_err(refused)?;
        let what = request.what.name();
        if by == Actor::Agent && self.sessions().stopped(session_id) {
            let error = refused(STOPPED);
            self.log_capture(session_id, by, what, &Err(error.clone()));
            return Err(error);
        }

        let bound = self.bind_read(session_id, by).await.map_err(refused)?;
        let _flight = self.sessions().begin(session_id, by);
        let mut cancel = self.sessions().cancel_signal(session_id);
        cancel.borrow_and_update();
        let work = async {
            tokio::time::timeout(CAPTURE_LIMIT, bound.capture_inner(session_id, &request, &target))
                .await
                .unwrap_or_else(|_| {
                    Err(failed(
                        "the capture took too long and was stopped; files written so far stay in the capture folder",
                    ))
                })
        };
        let result = if by == Actor::Agent {
            tokio::select! {
                result = work => result,
                _ = cancel.changed() => Err(ActError { capture_block: None, protected_values_withheld: None, outcome: Outcome::Stopped, text: STOPPED.to_owned() }),
            }
        } else {
            work.await
        };
        self.log_capture(session_id, by, what, &result);
        result
    }

    fn log_capture(
        &self,
        session_id: &str,
        by: Actor,
        what: &str,
        result: &Result<Value, ActError>,
    ) {
        let (outcome, detail) = match result {
            Ok(answer) => (
                Outcome::Done,
                answer["folder"].as_str().unwrap_or("captured").to_owned(),
            ),
            Err(e) => (e.outcome, e.text.clone()),
        };
        self.sessions()
            .end(session_id, by, "capture", what, outcome, &detail);
    }

    /// The checks before a capture writes anything: a request whose output is pixels is refused as a screenshot
    /// is, and a layout capture needs a usable view ([`check_view`]).
    async fn admit(&self, session_id: &str, request: &Request) -> Result<(), ActError> {
        if takes_pixels(request) {
            self.service()
                .pixels_admitted(session_id)
                .map_err(|error| match error {
                    vw_preview::PreviewError::ScreenshotHidden(ref block) => ActError {
                        capture_block: Some(block.kind),
                        protected_values_withheld: None,
                        outcome: Outcome::Refused,
                        text: error.to_string(),
                    },
                    other => failed(other.to_string()),
                })?;
        }
        if needs_layout(request) {
            let view = self
                .service()
                .layout_viewport(session_id)
                .await
                .map_err(|error| failed(error.to_string()))?;
            check_view(view, request)?;
        }
        Ok(())
    }

    async fn capture_inner(
        &self,
        session_id: &str,
        request: &Request,
        target: &Target<'_>,
    ) -> Result<Value, ActError> {
        #[cfg(feature = "e2e-script")]
        self.before_image().await;
        let head = shot::step(self, session_id, "metrics", 0.0)
            .await
            .map_err(refused)?;
        self.admit(session_id, request).await?;
        let (folder_stamp, captured_at) = stamp(SystemTime::now());
        let host = url::Url::parse(&head.url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        let name_source = folder_name_source(&head.title, &head.url, &host);
        let mut previous = None;
        let folder = match request.what {
            What::After => {
                let before = self.captures().before(session_id).ok_or_else(|| {
                    refused(
                        "there is no before shot in this chat: capture with what \"before\" first",
                    )
                })?;
                let folder =
                    Folder::reopen(target.session_folder, &before.relative).map_err(failed)?;
                previous = Some(before_record(&folder, &before.url)?);
                folder
            }
            What::Before => Folder::create(
                target.session_folder,
                &format!("{}-compare", slug(&name_source)),
                &folder_stamp,
            )
            .map_err(failed)?,
            _ => Folder::create(target.session_folder, &name_source, &folder_stamp)
                .map_err(failed)?,
        };
        let name = safe_name(&head.title, 80)
            .or_else(|| safe_name(&host, 80))
            .unwrap_or_else(|| "Captured page".to_owned());
        let mut run = Run {
            folder,
            source: Source {
                url: head.url.clone(),
                captured_at: captured_at.clone(),
                name,
            },
            summary: previous
                .as_ref()
                .and_then(|(_, record)| record["summary"].as_object().cloned())
                .unwrap_or_default(),
            skipped: previous
                .as_ref()
                .and_then(|(_, record)| record["skipped"].as_array().cloned())
                .unwrap_or_default(),
            page_words: vec![("title".to_owned(), head.title.clone())],
            fonts: Vec::new(),
            before: None,
        };
        let what = request.what;
        let one_file = what == What::Page
            && request
                .formats
                .iter()
                .any(|f| matches!(f, PageFormat::Pdf | PageFormat::SingleHtml));
        if one_file {
            return self
                .capture_one_file(session_id, request, target, run)
                .await;
        }
        let all = matches!(what, What::Bundle);
        self.capture_parts(session_id, &mut run, target, what).await;
        if all || what == What::Page {
            for format in &request.formats {
                if let Err(e) = self
                    .capture_page(session_id, &mut run, *format, request.method)
                    .await
                {
                    run.skip(&format!("page {format:?}").to_lowercase(), &e);
                }
            }
        }
        if what == What::Before || what == What::After {
            self.capture_compare(session_id, &mut run, what, request.method)
                .await
                .map_err(failed)?;
        }
        save_record(
            &mut run,
            previous.as_ref().map(|(bytes, _)| bytes.as_slice()),
            what,
            &head.title,
            &captured_at,
        )?;
        run.publish(self.captures(), session_id, what);
        Ok(answer(&run, what))
    }

    async fn capture_parts(
        &self,
        session_id: &str,
        run: &mut Run,
        target: &Target<'_>,
        what: What,
    ) {
        let all = what == What::Bundle;
        if (all || what == What::Design)
            && let Err(e) = self.capture_design(session_id, run).await
        {
            run.skip("design", &e);
        }
        if (all || what == What::Structure)
            && let Err(e) = self.capture_structure(session_id, run).await
        {
            run.skip("structure", &e);
        }
        if (all || what == What::Media)
            && let Err(e) = self.capture_media(session_id, run, target).await
        {
            run.skip("media", &e);
        }
    }
}
