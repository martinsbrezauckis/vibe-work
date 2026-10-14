//! The project handbook part of the command API: the requests both clients send inside
//! `Command::Pages` (the Project tab, the Projects overview), and what comes back. The model and its rules are in
//! vw-pages; the handler is `pages`.
//!
//! Depends on: vw-pages (views), vw-preview (`Rect`), serde. Called by: `command`, `pages`, the UI through the
//! generated TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - A request names a session; its project is the session's own folder (never a path from the client). The two
//!   overview requests are the exception: `overview` names nothing, `projectHub` names a project that must be one
//!   of the folders the core knows from Vibe's session data (any other path is refused).
//! - Nothing here writes into a repository (Handbook Format 1.2 §8: the app reads handbooks, never writes them).
//! - Page paths are relative to the project's pages folder and checked (`PagePath`) before any file is touched.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use vw_pages::{
    AnswerValue, Comment, Hub, PageAnswers, PageFacts, PagesSettings, PickedSection, ProjectCard,
    ViewerView, VisualReport,
};
use vw_preview::Rect;

/// One handbook request. From the Debug API origin, `open`, `plan`, `back`, `forward`, `reload`, `pickSection`,
/// `addComment`, `deleteComment`, `markSent` and `setSettings` need the native confirmation; reads (`hub`, `page`,
/// `overview`, `projectHub`, `settings`), `layout`, `cancelPick`, `clearExternal` and `screenshot` run directly. From the app's own window everything runs
/// directly (trusted surface).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PagesRequest {
    /// The session project's live hub: pages by kind, the manifest, the validator's findings (reads only).
    Hub {
        /// The session.
        session_id: String,
    },
    /// The Projects overview: one card per project Vibe Work knows (manifest head, active plans; heads only).
    Overview {},
    /// One known project's live hub without a session (the overview's health line and detail pane).
    ProjectHub {
        /// The project folder, as an `overview` card names it.
        project: String,
    },
    /// One page's facts and comments.
    Page {
        /// The session.
        session_id: String,
        /// Path inside the pages folder.
        path: String,
    },
    /// Shows a page (or the hub, without `path`) in the session's viewer.
    Open {
        /// The session.
        session_id: String,
        /// Path inside the pages folder; absent: the hub.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        path: Option<String>,
        /// A section or step id to scroll to (a click in the plan checklist).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        anchor: Option<String>,
    },
    /// Shows the current plan (the newest plan page) and answers its facts.
    Plan {
        /// The session.
        session_id: String,
    },
    /// History back in the viewer.
    Back {
        /// The session.
        session_id: String,
    },
    /// History forward.
    Forward {
        /// The session.
        session_id: String,
    },
    /// Reload.
    Reload {
        /// The session.
        session_id: String,
    },
    /// Where the window shows the viewer (the tab's page area), or nothing.
    Layout {
        /// The shown session (absent: hide).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
        /// The page area in the main window's CSS pixels (absent: hide).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        rect: Option<Rect>,
    },
    /// Lets the user point at a section of the shown page (the selection, else the next click).
    PickSection {
        /// The session.
        session_id: String,
    },
    /// Cancels a running pick.
    CancelPick {},
    /// Adds a comment on a section of a page.
    AddComment {
        /// The session.
        session_id: String,
        /// The page.
        path: String,
        /// The section id (must exist in the page file).
        section: String,
        /// The comment.
        text: String,
        /// The text the user selected.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        quote: Option<String>,
    },
    /// Deletes a comment.
    DeleteComment {
        /// The session.
        session_id: String,
        /// The comment id.
        id: String,
    },
    /// Marks comments as put into the composer (sent with the next message).
    MarkSent {
        /// The session.
        session_id: String,
        /// Comment ids.
        ids: Vec<String>,
    },
    /// The project's Pages settings.
    Settings {
        /// The session.
        session_id: String,
    },
    /// Changes the project's Pages settings.
    SetSettings {
        /// The session.
        session_id: String,
        /// The new settings.
        settings: PagesSettings,
    },
    /// Forgets the last refused external link of the viewer.
    ClearExternal {
        /// The session.
        session_id: String,
    },
    /// A PNG of the viewer (base64, at most 1280 px wide and 1 MiB).
    Screenshot {
        /// The session.
        session_id: String,
    },
    ///  (Format 1.2 §6.6): the inputs of one page with the user's answers, or of every page with inputs or
    /// answers (without `path`).
    Answers {
        /// The session.
        session_id: String,
        /// Path inside the handbook folder; absent: every page with inputs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        path: Option<String>,
    },
    /// Sets the answer to one input (checked against the page file).
    SetAnswer {
        /// The session.
        session_id: String,
        /// The page.
        path: String,
        /// The input id.
        input: String,
        /// The answer.
        value: AnswerValue,
    },
    /// Removes the answer to one input.
    ClearAnswer {
        /// The session.
        session_id: String,
        /// The page.
        path: String,
        /// The input id.
        input: String,
    },
    /// Marks every answer of a page as put into the composer ("Send to Vibe").
    MarkAnswersSent {
        /// The session.
        session_id: String,
        /// The page.
        path: String,
    },
    /// The app's theme, which the viewer passes to every page (`light` or `dark`; Format 1.2 §6.4).
    Theme {
        /// `light` or `dark`.
        theme: String,
    },
    /// The current plan's facts without opening it (the plan checklist); the core then watches its file and sends
    /// `pagesChanged` for the session when it changes.
    CurrentPlan {
        /// The session.
        session_id: String,
    },
    /// Exports a page as one self-contained HTML file. Without `destination`: the file's text comes back. With it
    /// (Debug API only; the window saves through the native save dialog): written there.
    ExportPage {
        /// The session.
        session_id: String,
        /// The page.
        path: String,
        /// An absolute file path to write.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        destination: Option<String>,
    },
    /// Lays a page out at desktop and phone width in light and dark and reports what is wrong (cached while the
    /// page and its stylesheet are unchanged).
    VisualCheck {
        /// The session.
        session_id: String,
        /// The page.
        path: String,
    },
}

/// One project's live hub (`projectHub`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectHub {
    /// The project folder.
    pub project: String,
    /// Its shown name (last folder name).
    pub name: String,
    /// Its hub (`None`: no handbook folder yet).
    pub hub: Option<Hub>,
}

/// What a Pages request returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PagesResult {
    /// `hub`.
    Hub {
        /// The project folder.
        project: String,
        /// Its shown name.
        name: String,
        /// The project's settings (handbook folder, scripts).
        settings: PagesSettings,
        /// The hub (`None`: no handbook folder yet). Boxed: the result enum stays small.
        hub: Option<Box<Hub>>,
        /// Comments per page path.
        comment_counts: BTreeMap<String, u32>,
        /// The session's viewer.
        viewer: Option<ViewerView>,
        /// Current visual check reports by page (pages not checked since they changed are absent).
        #[serde(default)]
        visual: BTreeMap<String, VisualReport>,
    },
    /// `overview`.
    Overview {
        /// Every known project's card, by name.
        projects: Vec<ProjectCard>,
        /// How long the core took to read the heads, ms.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        elapsed_ms: u64,
    },
    /// `projectHub`.
    ProjectHub {
        /// The project and its hub (boxed: the result enum stays small).
        project: Box<ProjectHub>,
    },
    /// `page`, `plan`.
    Page {
        /// The page.
        path: String,
        /// What the page file says (boxed: the result enum stays small).
        facts: Box<PageFacts>,
        /// Comments on sections that exist.
        attached: Vec<Comment>,
        /// Comments whose section is gone.
        detached: Vec<Comment>,
        /// The session's viewer.
        viewer: Option<ViewerView>,
    },
    /// `open`, `back`, `forward`, `reload`, `clearExternal`.
    Viewer {
        /// The session's viewer after the request.
        viewer: Option<ViewerView>,
    },
    /// `pickSection`.
    Picked {
        /// The section, checked against the page file.
        picked: PickedSection,
    },
    /// `addComment`.
    Comment {
        /// The stored comment.
        comment: Comment,
    },
    /// `markSent`.
    Comments {
        /// The comments marked.
        comments: Vec<Comment>,
    },
    /// `settings`, `setSettings`.
    Settings {
        /// The settings.
        settings: PagesSettings,
    },
    /// `screenshot`.
    Screenshot {
        /// PNG, standard base64.
        png_base64: String,
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// `answers`, `setAnswer`, `clearAnswer`, `markAnswersSent`.
    Answers {
        /// The handbook folder, relative to the project (`docs/handbook`).
        folder: String,
        /// The pages asked for, in hub order.
        pages: Vec<PageAnswers>,
    },
    /// `currentPlan`.
    CurrentPlan {
        /// The plan page (`None`: the project has no plan page).
        path: Option<String>,
        /// Its facts (boxed: the result enum stays small).
        facts: Option<Box<PageFacts>>,
    },
    /// `exportPage`.
    Exported {
        /// A file name to suggest.
        name: String,
        /// Size of the file in bytes.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        bytes: u64,
        /// Files inlined.
        inlined: u32,
        /// References left out, plain words.
        left_out: Vec<String>,
        /// Where it was written (`destination`, or the native save dialog's choice).
        saved_to: Option<String>,
        /// The file's text (without `destination`).
        html: Option<String>,
    },
    /// `visualCheck`.
    Visual {
        /// The report.
        report: VisualReport,
    },
    /// `layout`, `cancelPick`, `deleteComment`, `theme`.
    Done {},
}
