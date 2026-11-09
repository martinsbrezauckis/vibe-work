//! Native frame observations and document-bound ref input. Depends on preview frame evaluation and models.
//! Called by the browser driver; stale handles and navigation never fall back to another document.

#[path = "frame_choices.rs"]
mod frame_choices;

use crate::{
    Browser,
    model::{PageModel, parse_answer},
    scripts,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use vw_preview::MountId;

/// A document-scoped native frame address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameRef {
    /// Host-held ancestor geometry and node identities.
    #[serde(skip)]
    pub(crate) ancestors: Vec<Ancestor>,
    /// Native frame handle within this mount.
    pub id: String,
    /// Per-document nonce; same-URL reloads invalidate it.
    pub document: String,
    /// Origin of the document that owns the element, without its credentials or query.
    pub origin: String,
    /// Full address retained only for document validation, never returned to the agent.
    #[serde(skip)]
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ancestor {
    pub id: String,
    pub document: String,
    /// The ancestor document's address (its document guard compares it).
    pub url: String,
    pub node: u64,
    pub bounds: RefBox,
    pub content: RefBox,
    pub viewport: (u32, u32),
    pub child_viewport: (u32, u32),
}

/// Viewport rectangle in top-level CSS pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefBox {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

#[derive(Deserialize)]
struct Child {
    /// Position in the parent's `window` list; -1 when the element's window is not among the first 64.
    index: i64,
    visible: bool,
    x: f64,
    y: f64,
    sx: f64,
    sy: f64,
    node: u64,
    bounds: RefBox,
    content: RefBox,
}

#[derive(Deserialize)]
struct Observation {
    model: PageModel,
    origin: String,
    path: Vec<usize>,
    children: Vec<Child>,
    document: String,
    /// The frame's child windows (`window.length`, at most 64), whether or not their elements are listed.
    windows: usize,
    /// Frames inside the document's open shadow roots (at most 64): no child windows, so never read.
    shadowed: usize,
    width: f64,
    height: f64,
}

/// Answer of a read whose main frame never got a native address within the wait: its document is not committed yet.
const NO_PAGE_ADDRESS: &str =
    "the page is still loading (its document has no address yet): read it again in a moment";

/// Answer of a read whose main frame holds a document the page made itself (inline, `data:`, `blob:`): such a
/// document is never a page of its own.
const INLINE_PAGE: &str =
    "this page is an inline frame document, which this browser does not open as a page";

/// How long a child frame between documents (or removed while read) is observed again before it is left out. The
/// main frame keeps the whole page-swap wait.
const CHILD_WAIT: Duration = Duration::from_secs(2);

/// The longest address an observation keeps (`observe.js`); a longer `data:` address is compared by this prefix.
const MAX_ADDRESS: usize = 2000;

/// Pause before the frames are observed again (a main frame without a native address, a child between documents).
const ADDRESS_RETRY: Duration = Duration::from_millis(300);

/// A main frame whose document still names another origin than its native address at the end of the page-swap wait.
const MAIN_MOVED: &str = "frame address differs from its native origin";

/// Why a child frame was left out of an observation, as the agent reads it. Fixed texts, never page data.
const UNADDRESSED: &str = "a frame has no address yet";
const SWITCHING: &str = "a frame was changing documents while it was read";
const NOT_READ_HERE: &str = "a frame shows a document this browser does not read here";
const UNCHECKED: &str = "a frame's address or origin could not be checked";
pub(crate) const UNREADABLE: &str = "a frame could not be read";

/// One observation's frames, sorted by frame path: native handle, main-frame flag, observation.
type Frames = Vec<(String, bool, Observation)>;

/// Why a child frame was left out, with its frame path when its answer names one. A reason accounts only for the
/// frame at its own path; one without a path accounts for no frame.
type Omitted = (&'static str, Option<Vec<usize>>);

/// What a frame's document takes its origin from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Its own native address (`http`/`https`, or another scheme the read policy decides on).
    Addressed,
    /// The document that created it: an `about:blank` or `about:srcdoc` frame, or one whose parent's script
    /// rewrote it (`document.open` gives it the parent's address while the native one stays `about:blank`).
    Inherited,
    /// None (a `data:` document is opaque); read under its parent's rule, so its parent must have been read.
    Opaque,
}

/// One pass over the frames' answers.
enum Pass {
    /// The frames to merge, and why the others were left out.
    Done(
        Vec<(String, bool, Observation, Kind, Option<String>)>,
        Vec<Omitted>,
    ),
    /// The main frame has no native address yet.
    MainUnaddressed,
    /// The main frame's native address names another origin than its document: the frame moved on (a link to
    /// another site, a redirect) before its old document stopped answering. Observe again while the wait allows.
    MainMoved,
    /// A child frame answered from between two documents: observe again while the wait allows.
    Switching,
}

impl Browser {
    /// Observes every frame of the mount in the engine's isolated world and merges them into one page model.
    ///
    /// No child frame can fail the read: one that cannot be read or checked now is left out of this observation
    /// (none of its refs or text, so nothing can act on or fill into it) and its reason is listed. A frame without
    /// a native address (`WebKitGTK` names a frame's initial empty document so) or one whose script fails is left out
    /// at once; a child that answers from between two documents, or a frame removed while the frames were evaluated,
    /// is observed again for at most 2 s first. A main frame without a native address is observed again for at most
    /// the page-swap wait, then the read answers that the page is still loading; one whose document names another
    /// origin than its native address is observed again for the same wait, then the read fails (never read). The page-reported address never
    /// stands in for the native one. An inline (`about:srcdoc`) or `about:blank` child takes its parent's verified
    /// origin when its own security origin is that one, else it has none (`null`: a sandboxed frame, readable but
    /// never a vault target); a `data:` child never has one; a `blob:` child has the origin its native address names.
    pub(crate) async fn observe_frames(
        &self,
        session: &str,
        mount: &MountId,
        page: &str,
        script: &str,
    ) -> Result<Option<PageModel>, String> {
        let source = scripts::in_frame(script, &json!({"page":page,"observe":true}));
        let start = tokio::time::Instant::now();
        loop {
            let waited = start.elapsed();
            let last = waited >= crate::page::PAGE_SWAP_WAIT;
            let child_last = waited >= CHILD_WAIT;
            let answers = match self
                .service
                .evaluate_frames_on_mount(mount, None, source.clone(), None)
                .await
            {
                Ok(Some(answers)) => answers,
                Ok(None) => return Ok(None),
                // A frame removed between the engine's frame list and its evaluation fails that evaluation.
                Err(error) if !child_last && detached(&error.to_string()) => {
                    tokio::time::sleep(ADDRESS_RETRY).await;
                    continue;
                }
                Err(error) => return Err(error.to_string()),
            };
            match self.frame_observations(session, page, answers, child_last)? {
                Pass::Done(mut frames, mut unread) => {
                    frames.sort_by(|left, right| left.2.path.cmp(&right.2.path));
                    let frames = inherit(frames, &mut unread);
                    return merge(&frames, unread).map(Some);
                }
                Pass::MainUnaddressed if last => return Err(NO_PAGE_ADDRESS.into()),
                Pass::MainMoved if last => return Err(MAIN_MOVED.into()),
                Pass::MainUnaddressed | Pass::MainMoved | Pass::Switching => {
                    tokio::time::sleep(ADDRESS_RETRY).await;
                }
            }
        }
    }

    /// Checks each frame's answer against its native address. A child that fails a check is left out with its
    /// reason; the main frame's failure fails the read. `last`: no time is left to observe again.
    #[allow(clippy::too_many_lines)]
    fn frame_observations(
        &self,
        session: &str,
        page: &str,
        answers: Vec<vw_preview::FrameAnswer>,
        last: bool,
    ) -> Result<Pass, String> {
        let mut frames = Vec::new();
        let mut unread = Vec::new();
        // Children whose native address is not the one their document reports: (path, document nonce).
        let mut moved = Vec::new();
        let mut bytes = 0usize;
        for answer in answers.into_iter().take(64) {
            bytes = bytes.saturating_add(answer.value.len());
            if bytes > crate::model::MAX_ANSWER {
                return Err("frame observations exceeded the size limit".into());
            }
            let main = answer.main;
            // A child's failed check leaves that frame out; the main frame's fails the read.
            macro_rules! child {
                ($reason:expr, $error:expr, $path:expr) => {{
                    if main {
                        return Err($error.into());
                    }
                    unread.push(($reason, Some($path.clone())));
                    continue;
                }};
            }
            // Without its native address a frame's document cannot be checked: nothing of it is used but the path
            // its answer names, which the reason accounts for.
            let Ok(native) = url::Url::parse(&answer.url) else {
                if main {
                    return Ok(Pass::MainUnaddressed);
                }
                let path = parse_answer::<Observation>(&answer.value)
                    .ok()
                    .map(|o| o.path);
                unread.push((UNADDRESSED, path));
                continue;
            };
            if main
                && (native.as_str() == "about:srcdoc" || matches!(native.scheme(), "data" | "blob"))
            {
                return Err(INLINE_PAGE.into());
            }
            // A removed frame's handle answering from its detached document is no frame of the page.
            if !main && answer.value == vw_preview::DETACHED_FRAME {
                continue;
            }
            let mut observation: Observation = match parse_answer(&answer.value) {
                Ok(observation) => observation,
                Err(reason) if main => return Err(reason),
                Err(reason) if reason.starts_with("the page changed while it was read") => {
                    if !last {
                        return Ok(Pass::Switching);
                    }
                    unread.push((SWITCHING, None));
                    continue;
                }
                Err(_) => {
                    unread.push((UNREADABLE, None));
                    continue;
                }
            };
            if answer
                .origin
                .as_ref()
                .is_some_and(|origin| *origin != observation.origin)
            {
                child!(
                    UNCHECKED,
                    "frame security origin changed in the page script",
                    observation.path
                );
            }
            let Ok(reported) = url::Url::parse(&observation.model.url) else {
                child!(UNCHECKED, "frame address unavailable", observation.path);
            };
            let inherited = matches!(native.as_str(), "about:blank" | "about:srcdoc");
            // `document.open` from the parent gives an `about:blank` frame the parent's address; its origin is
            // proven against the parent's below, never by that address.
            let opened = native.as_str() == "about:blank"
                && matches!(reported.scheme(), "http" | "https")
                && !main;
            let differs = if opened {
                false
            } else if matches!(native.scheme(), "http" | "https") {
                native.origin() != reported.origin()
            } else if native.scheme() == "data" {
                !same_data(native.as_str(), &observation.model.url)
            } else {
                native != reported
            };
            if differs {
                if main {
                    return Ok(Pass::MainMoved);
                }
                // A child's native address can move on before its old document answers, and `WebKit` on macOS also
                // keeps an old handle of a frame (with its earlier address) that answers from the frame's document.
                moved.push((observation.path.clone(), observation.document.clone()));
                continue;
            }
            if !inherited
                && observation.origin != "null"
                && observation.origin != native.origin().ascii_serialization()
            {
                child!(
                    UNCHECKED,
                    "frame security origin differs from its native address",
                    observation.path
                );
            }
            if (main && inherited) || native.scheme() == "data" {
                // A top-level blank page, and every `data:` document, has no origin a vault fill could match.
                observation.origin = "null".into();
            }
            if let Err(reason) =
                self.service
                    .frame_reading_allowed(session, page, &observation.model.url)
            {
                child!(NOT_READ_HERE, reason, observation.path);
            }
            if observation.path.len() > 8 || observation.document.len() > 80 {
                child!(UNREADABLE, "invalid frame identity", observation.path);
            }
            let kind = if inherited && !main {
                Kind::Inherited
            } else if native.scheme() == "data" {
                Kind::Opaque
            } else {
                Kind::Addressed
            };
            observation.model.display_url = Some(native.to_string());
            let reported_origin = opened.then(|| reported.origin().ascii_serialization());
            frames.push((answer.id, main, observation, kind, reported_origin));
        }
        for (path, document) in moved {
            // Another handle of that same document passed every check: this one adds nothing.
            if frames
                .iter()
                .any(|(_, _, o, _, _)| o.path == path && o.document == document)
            {
                continue;
            }
            if !last {
                return Ok(Pass::Switching);
            }
            unread.push((UNCHECKED, Some(path)));
        }
        Ok(Pass::Done(frames, unread))
    }

    pub(crate) fn frame_for_script(&self, session: &str, script: &str) -> Option<FrameRef> {
        let request = script.split_once("const REQ = ")?.1.split_once(";\n")?.0;
        let request: serde_json::Value = serde_json::from_str(request).ok()?;
        let reference = request["ref"].as_str()?;
        self.current_seen(session)?
            .items
            .iter()
            .find(|r| r.id == reference)?
            .frame_id
            .clone()
    }

    pub(crate) async fn run_frame(
        &self,
        session: &str,
        mount: &MountId,
        frame: &FrameRef,
        script: &str,
        allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<String, String> {
        self.run_frame_as(session, mount, frame, script, allowed, false)
            .await
    }

    /// [`Self::run_frame`]; `input` runs an act's own input as the person's gesture
    /// (`PreviewService::evaluate_input_frames_on_mount`).
    pub(crate) async fn run_frame_as(
        &self,
        session: &str,
        mount: &MountId,
        frame: &FrameRef,
        script: &str,
        allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
        input: bool,
    ) -> Result<String, String> {
        let page = self
            .service
            .page_accessor(session)
            .ok_or("the frame's page is closed")?;
        self.service
            .frame_reading_allowed(session, &page, &frame.url)?;
        let guarded = scripts::in_frame(
            script,
            &json!({"page":page,"document":frame.document,"url":frame.url}),
        );
        let answers = if input {
            self.service
                .evaluate_input_frames_on_mount(mount, Some(frame.id.clone()), guarded, allowed)
                .await
        } else {
            self.service
                .evaluate_frames_on_mount(mount, Some(frame.id.clone()), guarded, allowed)
                .await
        };
        let answers = answers
            .map_err(|e| e.to_string())?
            .ok_or("frame input is unavailable")?;
        let mut answers = answers.into_iter();
        let answer = answers.next().ok_or("frame is stale or detached")?;
        if answer.id != frame.id || answers.next().is_some() {
            return Err("frame identity changed".into());
        }
        Ok(answer.value)
    }
}

/// One `data:` document in two spellings: the native address (`WebKit` on macOS percent-encodes it) and the one
/// the document reports, which the observation keeps to [`MAX_ADDRESS`] characters.
fn same_data(native: &str, reported: &str) -> bool {
    let (native, reported) = (percent_decoded(native), percent_decoded(reported));
    if reported.chars().count() >= MAX_ADDRESS {
        native.starts_with(&reported)
    } else {
        native == reported
    }
}

/// `text` with every `%XX` escape decoded (invalid UTF-8 replaced).
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| char::from(b).to_digit(16);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push(u8::try_from(high * 16 + low).unwrap_or(b'?'));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A frame path step as the page script reports a child's position.
fn pos(index: usize) -> i64 {
    i64::try_from(index).unwrap_or(-1)
}

/// True for the engines' answer when a frame went away between their frame list and its evaluation.
fn detached(error: &str) -> bool {
    error.contains("frame document changed, detached, or could not be evaluated")
        || error.contains("frame is stale or detached")
}

/// Gives each inherited child (sorted by path, parents first) its parent's verified origin when its own security
/// origin is that one; else `null` (opaque: a sandboxed frame). A child whose parent was left out is left out too, and
/// a rewritten `about:blank` frame whose address is not on that origin is left out as unchecked.
fn inherit(
    frames: Vec<(String, bool, Observation, Kind, Option<String>)>,
    unread: &mut Vec<Omitted>,
) -> Frames {
    let mut out: Frames = Vec::with_capacity(frames.len());
    for (id, main, mut observation, kind, reported) in frames {
        let has_parent = |path: &[usize]| {
            path.split_last()
                .is_some_and(|(_, parent)| out.iter().any(|(_, _, o)| o.path == parent))
        };
        if kind == Kind::Opaque && !has_parent(&observation.path) {
            continue;
        }
        if kind == Kind::Inherited {
            let parent = observation.path.split_last().and_then(|(_, path)| {
                out.iter()
                    .find(|(_, _, o)| o.path == path)
                    .map(|(_, _, o)| o.origin.clone())
            });
            let Some(parent) = parent else {
                continue;
            };
            let own = std::mem::take(&mut observation.origin);
            let proven = own != "null" && own == parent;
            if reported.is_some() && (!proven || reported.as_deref() != Some(own.as_str())) {
                unread.push((UNCHECKED, Some(observation.path)));
                continue;
            }
            observation.origin = if proven { own } else { "null".into() };
        }
        // Several handles of one frame (old handles kept by `WebKit` on macOS) answer from the same document: it is
        // read once. Two different documents claiming one frame position are left out, as unchecked.
        if let Some(index) = out.iter().position(|(_, _, o)| o.path == observation.path) {
            if out[index].2.document != observation.document {
                out[index].2.document.clear();
            }
            continue;
        }
        out.push((id, main, observation));
    }
    unread.extend(
        out.iter()
            .filter(|(_, _, o)| o.document.is_empty())
            .map(|(_, _, o)| (UNCHECKED, Some(o.path.clone()))),
    );
    out.retain(|(_, _, o)| !o.document.is_empty());
    out
}

/// Whether the read frames hold a frame this observation neither reads nor accounts for: a child window of a read
/// frame with no read frame and no reason of its own at its path (on Linux, one sandboxed without scripts from its
/// creation never reaches the engine's frame registry), or any frame in an open shadow root (no child window, so
/// never read, whether or not a handle answered). Such a frame names the page's frames unreadable, whatever
/// reasons other frames gave.
fn left_out(frames: &[(String, bool, Observation)], unread: &[Omitted]) -> bool {
    frames.iter().any(|(_, _, o)| o.shadowed > 0)
        || frames
            .iter()
            .flat_map(|(_, _, o)| {
                (0..o.windows.min(64)).map(|index| {
                    let mut path = o.path.clone();
                    path.push(index);
                    path
                })
            })
            .any(|path| {
                !frames.iter().any(|(_, _, o)| o.path == path)
                    && !unread.iter().any(|(_, p)| p.as_ref() == Some(&path))
            })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn merge(
    frames: &[(String, bool, Observation)],
    unread: Vec<Omitted>,
) -> Result<PageModel, String> {
    let top = frames
        .iter()
        .position(|(_, main, o)| *main && o.path.is_empty())
        .ok_or("the main frame changed while it was read")?;
    let mut model = frames[top].2.model.clone();
    model.document.clone_from(&frames[top].2.document);
    model.text_page_count = Some(
        model
            .text_length
            .div_ceil(crate::observe::PAGE_CHARS as u64)
            .max(1),
    );
    model.viewport = Some((
        frames[top].2.width.round() as u32,
        frames[top].2.height.round() as u32,
    ));
    model.refs.clear();
    model.choice_refs.clear();
    model.covering_frames.clear();
    model.all_refs.clear();
    model.ref_count = 0;
    model.cross_origin_frames = 0;
    model.unread_frames.clear();
    let left_out = left_out(frames, &unread);
    let mut unread: Vec<_> = unread.into_iter().map(|(reason, _)| reason).collect();
    if left_out {
        unread.push(UNREADABLE);
    }
    for reason in unread {
        if !model.unread_frames.iter().any(|r| r == reason) {
            model.unread_frames.push(reason.to_owned());
        }
    }
    for (id, main, observation) in frames {
        let Some((x, y, sx, sy)) = transform(frames, &observation.path) else {
            continue;
        };
        let frame = FrameRef {
            ancestors: ancestry(frames, &observation.path).ok_or("frame ancestry changed")?,
            id: id.clone(),
            document: observation.document.clone(),
            origin: observation.origin.clone(),
            url: observation.model.url.clone(),
        };
        model.documents.push(frame.clone());
        frame_choices::append_main(&mut model, observation, &frame, *main);
        let visible = observation.path.iter().enumerate().all(|(depth, index)| {
            frames
                .iter()
                .find(|(_, _, o)| o.path == observation.path[..depth])
                .and_then(|(_, _, o)| o.children.iter().find(|c| c.index == pos(*index)))
                .is_some_and(|c| c.visible)
        });
        let prominent = *main || (visible && covers_viewport(frames, &observation.path));
        let mut choices = Vec::new();
        for (item, choice) in observation.model.refs.iter().map(|r| (r, false)).chain(
            observation
                .model
                .choice_refs
                .iter()
                .filter(|_| !main && prominent)
                .map(|r| (r, true)),
        ) {
            let mut item = item.clone();
            item.local_bounds = item.bounds;
            item.local_viewport = (observation.width as u32, observation.height as u32);
            item.frame_id = Some(frame.clone());
            if !main {
                item.frame = Some(true);
            }
            if !visible {
                item.visible = Some(false);
                item.priority = -1;
            } else if !prominent {
                item.priority = item.priority.min(3);
            }
            if let Some(rect) = &mut item.bounds {
                rect.x = (x + f64::from(rect.x) * sx).round() as i32;
                rect.y = (y + f64::from(rect.y) * sy).round() as i32;
                rect.width = (f64::from(rect.width) * sx).round() as i32;
                rect.height = (f64::from(rect.height) * sy).round() as i32;
            }
            if choice {
                choices.push(item);
            } else {
                model.refs.push(item);
            }
        }
        if !main && prominent {
            frame_choices::append_child(&mut model, observation, choices);
        }
        append_frame_content(&mut model, &observation.model, *main, visible);
    }
    if !crate::choices::unique(&model) {
        return Err(
            "frame refs collided, so this read's refs cannot be used; read the page again".into(),
        );
    }
    Ok(model)
}

fn append_frame_content(model: &mut PageModel, child: &PageModel, main: bool, visible: bool) {
    model
        .all_refs
        .extend(child.refs.iter().map(|r| r.id.clone()));
    model.ref_count = model.ref_count.saturating_add(child.ref_count);
    if !main {
        if visible {
            model.text_page_count = Some(
                model.text_page_count.unwrap_or(1).max(
                    child
                        .text_length
                        .div_ceil(crate::observe::PAGE_CHARS as u64),
                ),
            );
        }
        let text = if visible {
            &mut model.text
        } else {
            &mut model.hidden_text
        };
        if !child.text.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&child.text);
        }
        model.hidden_text.push('\n');
        model.hidden_text.push_str(&child.hidden_text);
        model.text_length = model.text_length.saturating_add(child.text_length);
    }
    model.truncated |= child.truncated;
}

/// Only a frame covering at least 40% of the top viewport can promote its overlays above main
/// form controls. This admits consent walls while excluding ordinary ads and banners. All boxes
/// come from parents, clipped through every ancestor: a child cannot enlarge its own allocation.
fn covers_viewport(frames: &[(String, bool, Observation)], path: &[usize]) -> bool {
    let Some((_, _, top)) = frames
        .iter()
        .find(|(_, main, o)| *main && o.path.is_empty())
    else {
        return false;
    };
    let area = top.width * top.height;
    if !area.is_finite() || area <= 0.0 {
        return false;
    }
    let (mut left, mut upper, mut right, mut bottom) = (0.0_f64, 0.0_f64, top.width, top.height);
    for (depth, index) in path.iter().enumerate() {
        let Some((x, y, sx, sy)) = transform(frames, &path[..depth]) else {
            return false;
        };
        let Some(child) = frames
            .iter()
            .find(|(_, _, o)| o.path == path[..depth])
            .and_then(|(_, _, o)| o.children.iter().find(|c| c.index == pos(*index)))
        else {
            return false;
        };
        let rect = child.content;
        let x = x + f64::from(rect.x) * sx;
        let y = y + f64::from(rect.y) * sy;
        left = left.max(x);
        upper = upper.max(y);
        right = right.min(x + f64::from(rect.width) * sx);
        bottom = bottom.min(y + f64::from(rect.height) * sy);
    }
    (right - left).max(0.0) * (bottom - upper).max(0.0) >= area * 0.4
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ancestry(frames: &[(String, bool, Observation)], path: &[usize]) -> Option<Vec<Ancestor>> {
    path.iter()
        .enumerate()
        .map(|(depth, index)| {
            let (id, _, parent) = frames.iter().find(|(_, _, o)| o.path == path[..depth])?;
            let child = parent
                .children
                .iter()
                .find(|child| child.index == pos(*index))?;
            let (_, _, inner) = frames.iter().find(|(_, _, o)| o.path == path[..=depth])?;
            Some(Ancestor {
                id: id.clone(),
                document: parent.document.clone(),
                url: parent.model.url.clone(),
                node: child.node,
                bounds: child.bounds,
                content: child.content,
                viewport: (parent.width as u32, parent.height as u32),
                child_viewport: (inner.width as u32, inner.height as u32),
            })
        })
        .collect()
}

fn transform(
    frames: &[(String, bool, Observation)],
    path: &[usize],
) -> Option<(f64, f64, f64, f64)> {
    let (mut x, mut y, mut sx, mut sy) = (0.0, 0.0, 1.0, 1.0);
    for (depth, index) in path.iter().enumerate() {
        let parent = &frames.iter().find(|(_, _, o)| o.path == path[..depth])?.2;
        let child = parent
            .children
            .iter()
            .find(|child| child.index == pos(*index))?;
        if [child.x, child.y, child.sx, child.sy]
            .iter()
            .any(|n| !n.is_finite())
            || child.sx <= 0.0
            || child.sy <= 0.0
        {
            return None;
        }
        x += child.x * sx;
        y += child.y * sy;
        sx *= child.sx;
        sy *= child.sy;
    }
    Some((x, y, sx, sy))
}
