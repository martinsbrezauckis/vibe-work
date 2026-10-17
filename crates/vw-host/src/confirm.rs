//! The confirmer the app hands to the core: a boxed function, so the Tauri crate can supply a
//! native-dialog implementation without `vw-host` depending on Tauri.
//!
//! Depends on: vw-gate (`Confirmer` and its request/answer types). Called by: [`crate::Host::start`]
//! (passes it to the core), `app` (builds the dialog one), tests (scripted ones).
//! Invariants:
//! - The default ([`SharedConfirmer::decline`]) fails closed: every request is declined.
//! - A confirmer is asked once per request and its answer is returned unchanged.
//! - Nothing in this module can be reached from the webview or the Debug API: they can only ask the
//!   core to request a confirmation; the answer comes from this function (a native dialog), so a
//!   remote client cannot approve its own request.
//! - A request is shown in full or not at all ([`dialog_text`]): callers supply complete plain sentences,
//!   including executable input. Methods and wire parameters never appear in the dialog. Clipping the text would let a
//!   client put a harmless start in view and the real command past the cut. The same dialog
//!   guards `session/shellCommand`.
//! - A request too long for one dialog (any requester: the agent's git and kill holds, a window action such as
//!   removing a copy with many files, a Debug API vault request) is never declined without a dialog (that told the
//!   caller "the user declined" while the user saw nothing, and made ordinary long requests fail silently). It is
//!   shown SHORTENED, never silently clipped: a first line says it is shortened and by how much, the start and the
//!   end of the text are shown with a marker in between naming the characters left out. It can be allowed only after
//!   the user paged through ALL of it in the same dialog ([`ShortStep`]: overview -> part 1..n -> Allow on the last
//!   part; Return never approves unseen text; "always" is never offered on it); one too long to page
//!   ([`MAX_PAGES`]) can only be refused, with a dialog that says so.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use vw_gate::{Confirmation, ConfirmationRequest, Confirmer, Requester};

/// Longest caller text a confirmation dialog shows. Longer requests are declined or paged, never clipped.
pub const DIALOG_PARAMS_MAX_CHARS: usize = 1200;

/// The first line of a dialog the window did not ask for (from the request's
/// structured `requester`, never from the wording of its summary).
pub const DEBUG_API_LINE: &str =
    "An external client (the Debug API), not a click in this window, asks for this.";
/// The first line of a dialog the agent of a session asked for.
pub const AGENT_LINE: &str = "The agent of a session, not a click in this window, asks for this.";

/// Caller text with hidden characters written as visible escapes. Paragraph breaks are supplied by
/// the caller; values embedded in prose must be quoted/escaped there rather than becoming instructions.
fn visible(text: &str) -> String {
    text.chars()
        .map(|c| {
            if (c.is_control() || vw_gate::is_hidden_char(c)) && !matches!(c, '\n' | ' ') {
                format!("\\u{{{:04x}}}", u32::from(c))
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// Who asks (when it is not the window), followed only by the caller's full plain explanation.
/// Wire methods and JSON stay private. Hidden characters are escaped, and paragraph breaks remain.
/// `None` for missing text, malformed browser-save metadata, or text longer than
/// [`DIALOG_PARAMS_MAX_CHARS`]. The caller must decline or use the agent's full-text pagination.
#[must_use]
pub fn dialog_text(request: &ConfirmationRequest) -> Option<String> {
    let text = request_text(request)?;
    if text.chars().count() > DIALOG_PARAMS_MAX_CHARS {
        return None;
    }
    Some(format!("{}{text}", lead(request)))
}

fn request_text(request: &ConfirmationRequest) -> Option<String> {
    if request.method == "vibe-work/captureSecret" {
        // Preserve the reviewed save schema: an unknown field cannot quietly disappear from consent.
        return capture_save_text(&request.params);
    }
    (!request.summary.trim().is_empty()).then(|| visible(&request.summary))
}

/// The capture payload contains only metadata, never the selected value. Show every field without JSON or
/// method names; the same conservative admission above still applies. Unknown fields fail closed so
/// a future payload change cannot silently omit something the user should approve.
fn capture_save_text(params: &serde_json::Value) -> Option<String> {
    let fields = [
        "session",
        "page",
        "element",
        "storeAs",
        "valueLength",
        "replaces",
    ];
    let object = params.as_object()?;
    if object.len() != fields.len() || object.keys().any(|key| !fields.contains(&key.as_str())) {
        return None;
    }
    // Quoted text is escaped before insertion: line breaks and hidden direction marks cannot turn
    // metadata into instructions. Unlike the old summary, no name or site is truncated.
    let quote = |key: &str| -> Option<String> {
        let text = params.get(key)?.as_str()?;
        Some(visible(&serde_json::to_string(text).ok()?))
    };
    let session = quote("session")?;
    let page = quote("page")?;
    let element = quote("element")?;
    let name = quote("storeAs")?;
    let length = params.get("valueLength")?.as_u64()?;
    let replacement = if params.get("replaces")?.as_bool()? {
        " This replaces the value already saved under that name."
    } else {
        ""
    };
    Some(format!(
        "Save the {length}-character value you selected in {element} on {page} to your vault as {name}?\n\n\
         This is for the session {session}.{replacement} The agent never sees the value; it can later ask to use it \
         in a command."
    ))
}

/// The default button of a shortened request: pages through its full text.
pub const SHOW_FULL: &str = "Show the full request";
/// The default button of a page of the full text that is not the last.
pub const NEXT_PART: &str = "Next part";
/// The only button of a request too long to page through here.
pub const CLOSE: &str = "Close";
/// Characters of the caller text a shortened overview shows from the start.
pub const SHORT_HEAD: usize = 700;
/// Characters of the caller text a shortened overview shows from the end.
pub const SHORT_TAIL: usize = 300;
/// Characters of the full text per page (a page always fits a dialog).
pub const PAGE_CHARS: usize = 1000;
/// Most pages offered; a longer request can only be refused here.
pub const MAX_PAGES: usize = 12;

/// Where a shortened request's dialog is: the overview, or page `n` (1-based) of the full text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortStep {
    /// The shortened overview (start, marker, end).
    Overview,
    /// One page of the full caller text.
    Page(usize),
}

/// What a pressed button of a shortened request leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortOutcome {
    /// Show this step next.
    Ask(ShortStep),
    /// The answer.
    Done(Confirmation),
}

/// The caller text of a request too long for one dialog, as characters (`None`: it fits, or it has no text).
fn long_text(request: &ConfirmationRequest) -> Option<Vec<char>> {
    let text = request_text(request)?;
    (text.chars().count() > DIALOG_PARAMS_MAX_CHARS).then(|| text.chars().collect())
}

/// The first line naming who asks, from the request's structured `requester` (empty for a click in this window).
fn lead(request: &ConfirmationRequest) -> String {
    match request.requester {
        // `Core` is never shown: the core answers its own requests (`vw-core-api` confirmer).
        Requester::Window | Requester::Core => String::new(),
        Requester::DebugApi => format!("{DEBUG_API_LINE}\n\n"),
        Requester::Agent => format!("{AGENT_LINE}\n\n"),
    }
}

/// How many pages the full caller text takes (`None` when the request is not a shortened one).
#[must_use]
pub fn full_pages(request: &ConfirmationRequest) -> Option<usize> {
    long_text(request).map(|text| text.len().div_ceil(PAGE_CHARS).max(1))
}

/// The text of `step` for a request too long to show in full; `None` for a request that fits (shown whole by
/// [`dialog_text`]) or a page out of range.
///
/// The overview: the requester line (none for a click in this window), a first line saying the request is shortened (total and hidden character counts)
/// and that it can be allowed only after reading it whole, the first [`SHORT_HEAD`] characters of the caller text,
/// a marker naming how many are left out, the last [`SHORT_TAIL`]. A page: "part k of n" and [`PAGE_CHARS`]
/// characters of that text, every character in exactly one page. Neither view exposes wire parameters.
#[must_use]
pub fn shortened_dialog_text(request: &ConfirmationRequest, step: ShortStep) -> Option<String> {
    let params = long_text(request)?;
    let lead = lead(request);
    let total = params.len();
    let pages = total.div_ceil(PAGE_CHARS).max(1);
    match step {
        ShortStep::Overview => {
            let hidden = total.saturating_sub(SHORT_HEAD + SHORT_TAIL);
            let head: String = params.iter().take(SHORT_HEAD).collect();
            let tail: String = params.iter().skip(total - SHORT_TAIL.min(total)).collect();
            let how = if pages <= MAX_PAGES {
                format!(
                    "To allow it, read it whole first: \"{SHOW_FULL}\" shows all of it in {pages} parts, and the last part \
                     offers Allow."
                )
            } else {
                format!(
                    "It is too long to read here ({pages} parts of {PAGE_CHARS} characters), so it can only be refused. \
                     Put long command arguments in a file and request a shorter command."
                )
            };
            Some(format!(
                "{lead}SHORTENED: this request is {total} characters long, more than this window shows; \
                 {hidden} characters in the middle are not shown here. {how}\n\n\
                 {head}\n\n[... {hidden} characters not shown here ...]\n\n{tail}"
            ))
        }
        ShortStep::Page(n) if (1..=pages).contains(&n) => {
            let chunk: String = params
                .iter()
                .skip((n - 1) * PAGE_CHARS)
                .take(PAGE_CHARS)
                .collect();
            let next = if n < pages {
                format!("\"{NEXT_PART}\" shows part {} of {pages}.", n + 1)
            } else {
                "This is the end of the request.".to_owned()
            };
            Some(format!(
                "{lead}The full request, part {n} of {pages}. {next}\n\n{chunk}"
            ))
        }
        ShortStep::Page(_) => None,
    }
}

/// The buttons of `step`, first to last; the first is the default (Return) on every OS (`dialog_buttons`). Return
/// never approves text the user has not been shown: the overview offers only [`SHOW_FULL`] (default) and Cancel (or
/// only [`CLOSE`] when too long to page), a page that is not the last offers [`NEXT_PART`] and Cancel, and only the
/// last page offers the plain approval (`Allow once` / `Continue`; never "always" on a shortened request).
#[must_use]
pub fn shortened_buttons(request: &ConfirmationRequest, step: ShortStep) -> Vec<String> {
    let pages = full_pages(request).unwrap_or(0);
    let (allow, refuse) = if request.always.is_some() {
        (once_label(request), DONT_ALLOW)
    } else {
        (CONTINUE, "Cancel")
    };
    match step {
        ShortStep::Overview if pages > MAX_PAGES => vec![CLOSE.to_owned()],
        ShortStep::Overview => vec![SHOW_FULL.to_owned(), refuse.to_owned()],
        ShortStep::Page(n) if n < pages => vec![NEXT_PART.to_owned(), refuse.to_owned()],
        ShortStep::Page(_) => vec![allow.to_owned(), refuse.to_owned()],
    }
}

/// Where a pressed button of `step` leads. Fails closed: anything but the step's forward label declines; the
/// approval counts only on the last page of a request short enough to page through.
#[must_use]
pub fn shortened_answer(
    request: &ConfirmationRequest,
    step: ShortStep,
    pressed: Option<&str>,
) -> ShortOutcome {
    let pages = full_pages(request).unwrap_or(0);
    let allow = if request.always.is_some() {
        once_label(request)
    } else {
        CONTINUE
    };
    let declined = ShortOutcome::Done(Confirmation::Declined);
    match (step, pressed) {
        (ShortStep::Overview, Some(SHOW_FULL)) if (1..=MAX_PAGES).contains(&pages) => {
            ShortOutcome::Ask(ShortStep::Page(1))
        }
        (ShortStep::Page(n), Some(NEXT_PART)) if n < pages => {
            ShortOutcome::Ask(ShortStep::Page(n + 1))
        }
        (ShortStep::Page(n), Some(label)) if n == pages && pages <= MAX_PAGES && label == allow => {
            ShortOutcome::Done(Confirmation::Approved)
        }
        _ => declined,
    }
}

/// The plain approval of a three-button consent, its first button.
pub const ALLOW_ONCE: &str = "Allow once";
/// The plain approval when it lasts until Vibe Work closes (an MCP server's key, whose connection needs it
/// on every request). The second button: "always" comes first there.
pub const ALLOW_UNTIL_CLOSE: &str = "Allow until Vibe Work closes";

/// The plain approval's label of a three-button consent for `request`.
fn once_label(request: &ConfirmationRequest) -> &'static str {
    if request.method == vw_core_api::MCP_KEY_DIALOG_METHOD {
        ALLOW_UNTIL_CLOSE
    } else {
        ALLOW_ONCE
    }
}
/// The last button of a three-button consent.
pub const DONT_ALLOW: &str = "Don't allow";
/// The approving button of a two-button confirmation.
pub const CONTINUE: &str = "Continue";

/// The labels of the dialog's buttons for `request`, first to last. The first one is the default (Return) on every
/// OS: tauri-plugin-dialog 2.8.1 hands them to rfd 0.16 in this order, and Windows' task dialog (`nDefaultButton` 0:
/// "the first button in the dialog will be the default"), macOS' `NSAlert` (the first added button has Return) and
/// GTK (the first added button holds the focus) all default to the first.
/// - A request that offers "always": `Allow once` / the request's `always` label / `Don't allow`.
/// - An MCP server's key ("always" is the recommended choice): the
///   `always` label / `Allow until Vibe Work closes` / `Don't allow`.
/// - An agent asking to close a user tab: `Keep` / `Close` (Return keeps the tab).
/// - Otherwise `Continue` / `Cancel`.
///
/// The pressed label decides the answer ([`dialog_answer`]), so the order never changes what a label means.
#[must_use]
pub fn dialog_buttons(request: &ConfirmationRequest) -> Vec<String> {
    if request.method == "vibe-work/browserCloseTab" {
        return vec!["Keep".to_owned(), "Close".to_owned()];
    }
    match &request.always {
        Some(always) if request.method == vw_core_api::MCP_KEY_DIALOG_METHOD => vec![
            always.clone(),
            ALLOW_UNTIL_CLOSE.to_owned(),
            DONT_ALLOW.to_owned(),
        ],
        Some(always) => vec![
            once_label(request).to_owned(),
            always.clone(),
            DONT_ALLOW.to_owned(),
        ],
        None => vec![CONTINUE.to_owned(), "Cancel".to_owned()],
    }
}

/// The decision a pressed button stands for (`pressed`: its label; `None`: closed, failed or no dialog). Fails
/// closed: anything that is not exactly an approving label of this request declines.
#[must_use]
pub fn dialog_answer(request: &ConfirmationRequest, pressed: Option<&str>) -> Confirmation {
    if request.method == "vibe-work/browserCloseTab" {
        return if pressed == Some("Close") {
            Confirmation::Approved
        } else {
            Confirmation::Declined
        };
    }
    match (pressed, request.always.as_deref()) {
        (Some(label), Some(always)) if label == always => Confirmation::ApprovedAlways,
        (Some(label), Some(_)) if label == once_label(request) => Confirmation::Approved,
        (Some(CONTINUE), None) => Confirmation::Approved,
        _ => Confirmation::Declined,
    }
}

type Answer = Pin<Box<dyn Future<Output = Confirmation> + Send>>;

/// A cloneable, type-erased confirmer.
#[derive(Clone)]
pub struct SharedConfirmer(Arc<dyn Fn(ConfirmationRequest) -> Answer + Send + Sync>);

impl SharedConfirmer {
    /// Wraps `ask`, which shows the request to the user and resolves to their decision.
    pub fn new<F, Fut>(ask: F) -> Self
    where
        F: Fn(ConfirmationRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Confirmation> + Send + 'static,
    {
        Self(Arc::new(move |request| Box::pin(ask(request))))
    }

    /// Declines every request (used when no dialog can be shown).
    #[must_use]
    pub fn decline() -> Self {
        Self::new(|_| std::future::ready(Confirmation::Declined))
    }
}

impl Confirmer for SharedConfirmer {
    fn confirm(&self, request: &ConfirmationRequest) -> impl Future<Output = Confirmation> + Send {
        (self.0)(request.clone())
    }
}
