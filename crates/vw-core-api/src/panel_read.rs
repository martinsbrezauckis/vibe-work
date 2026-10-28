//! The agent's `panel_read`: what the user has open beside the chat. The core
//! asks the window (`CoreEvent::PanelRead`), the window answers with its typed view of that session's side panel
//! (`files` `panelAnswer`, a [`PanelView`]), and the agent gets it bounded, with plain notes on how to use it.
//!
//! Depends on: the session store, `panel_view`, events. Called by: `agent_tools::tools` (the tool), `files` (the
//! window's answer).
//! Invariants:
//! - Read-only: nothing here changes the panel, an editor or a file.
//! - Only the credential's own session is read: an answer is taken only when it names the session the request was
//!   made for, so no window can hand one session's panel to another session's agent.
//! - The answer carries editor text and Files-tab state only ([`PanelView`] refuses unknown fields); Terminal and
//!   Browser are named, never copied. Every text is bounded.
//! - No window, or no answer in [`WAIT`]: a plain outcome that says so, never a guess.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::sync::oneshot;

use crate::core::Shared;
use crate::event::CoreEvent;
use crate::panel_view::{PanelEditor, PanelView};

/// How long the tool waits for the window.
const WAIT: Duration = Duration::from_secs(5);
/// Most editors reported.
const MAX_EDITORS: usize = 20;
/// Most characters of one editor's unsaved text.
const MAX_TEXT: usize = 128 * 1024;
/// Most characters of a selection.
const MAX_SELECTION: usize = 32 * 1024;
/// Most characters of a path, a label or the Files search text.
const MAX_LABEL: usize = 1024;
/// Most tabs named.
const MAX_TABS: usize = 40;

type Waiting = HashMap<String, (String, oneshot::Sender<PanelView>)>;

/// Requests waiting for the window, each with the session it is for.
#[derive(Default)]
pub(crate) struct Pending {
    next: AtomicU64,
    waiting: Mutex<Waiting>,
}

impl Pending {
    fn begin(&self, session: &str) -> (String, oneshot::Receiver<PanelView>) {
        let id = format!("panel-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (sender, receiver) = oneshot::channel();
        self.waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone(), (session.to_owned(), sender));
        (id, receiver)
    }

    fn forget(&self, id: &str) {
        self.waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
    }

    /// Hands the window's answer to the waiting tool, only when it names the request's own session. False: no such
    /// request waits (answered already, expired) or it belongs to another session.
    pub(crate) fn answer(&self, session: &str, id: &str, view: PanelView) -> bool {
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        if waiting.get(id).is_none_or(|(owner, _)| owner != session) {
            return false;
        }
        waiting
            .remove(id)
            .is_some_and(|(_, reply)| reply.send(view).is_ok())
    }
}

/// The tool: asks the window for this session's side panel and answers what it shows.
pub(crate) async fn tool(shared: &Shared, session: &str, args: &Value) -> Result<Value, String> {
    match args {
        Value::Null => {}
        Value::Object(map) if map.is_empty() => {}
        _ => {
            return Err(
                "panel_read takes no arguments: it reads this session's side panel.".to_owned(),
            );
        }
    }
    if shared.store.view(session).is_none() {
        return Err("this session is not open in Vibe Work right now".to_owned());
    }
    let (request_id, reply) = shared.panel_reads.begin(session);
    shared.emit(CoreEvent::PanelRead {
        request_id: request_id.clone(),
        session_id: session.to_owned(),
    });
    if let Ok(Ok(view)) = tokio::time::timeout(WAIT, reply).await {
        Ok(agent_answer(view))
    } else {
        shared.panel_reads.forget(&request_id);
        Ok(json!({
            "status": "noWindow",
            "notes": ["No Vibe Work window answered, so the side panel could not be read. Ask the user which file they mean."],
        }))
    }
}

/// At most `max` characters of `text`, and whether some were left out.
fn cut(text: &str, max: usize) -> (String, bool) {
    match text.char_indices().nth(max) {
        Some((at, _)) => (text[..at].to_owned(), true),
        None => (text.to_owned(), false),
    }
}

fn label(text: &str) -> String {
    cut(text, MAX_LABEL).0
}

fn editor_json(editor: PanelEditor) -> Value {
    let mut out = Map::new();
    out.insert("path".into(), json!(label(&editor.path)));
    out.insert("focused".into(), json!(editor.focused));
    if let Some(side) = editor.side.filter(|s| s == "left" || s == "right") {
        out.insert("side".into(), json!(side));
    }
    if let Some(saved) = editor.saved_version.filter(|s| s == "saved" || s == "disk") {
        out.insert("savedVersion".into(), json!(saved));
    }
    out.insert("unsaved".into(), json!(editor.unsaved));
    if let Some(text) = editor.unsaved_text {
        let (text, left_out) = cut(&text, MAX_TEXT);
        out.insert("unsavedText".into(), json!(text));
        if left_out {
            out.insert("unsavedTextCut".into(), json!(true));
        }
    }
    if let Some(selection) = editor.selection {
        let (text, left_out) = cut(&selection.text, MAX_SELECTION);
        let mut picked = json!({
            "startLine": selection.start_line,
            "endLine": selection.end_line.max(selection.start_line),
            "text": text,
        });
        if left_out {
            picked["textCut"] = json!(true);
        }
        out.insert("selection".into(), picked);
    }
    if let Some(line) = editor.cursor_line {
        out.insert("cursorLine".into(), json!(line));
    }
    if !editor.changed_lines.is_empty() {
        let lines: Vec<u32> = editor.changed_lines.into_iter().take(1000).collect();
        out.insert("changedLines".into(), json!(lines));
    }
    Value::Object(out)
}

/// The agent's answer for `view`: bounded fields plus notes on what they mean.
pub(crate) fn agent_answer(view: PanelView) -> Value {
    let tabs: Vec<String> = view.tabs.iter().take(MAX_TABS).map(|t| label(t)).collect();
    let unsaved = view.editors.iter().any(|e| e.unsaved_text.is_some());
    let editors: Vec<Value> = view
        .editors
        .into_iter()
        .take(MAX_EDITORS)
        .map(editor_json)
        .collect();
    let mut notes: Vec<&str> = Vec::new();
    if editors.is_empty() {
        notes.push(if view.open {
            "No file is open in an editor beside the chat. Ask the user which file they mean."
        } else {
            "Nothing is open beside the chat: no file and no editor. Ask the user which file they mean."
        });
    }
    if unsaved {
        notes.push(
            "unsavedText is the user's own text, not saved yet: the file on disk differs. Say so before you \
             change that file; your edit goes to the file on disk and the user sees it marked in the editor.",
        );
    }
    if tabs.iter().any(|t| t == "Terminal") {
        notes.push("Terminal is open: its processes are in processes_list.");
    }
    if tabs.iter().any(|t| t == "Browser") {
        notes.push("Browser is open: read its page with browser_read.");
    }
    let mut out = json!({
        "status": if editors.is_empty() && !view.open { "nothingOpen" } else { "read" },
        "panelOpen": view.open,
        "tabs": tabs,
        "shown": view.active.as_deref().map(label),
        "editors": editors,
        "notes": notes,
    });
    if let Some(files) = view.files {
        out["files"] = json!({
            "folder": files.folder.as_deref().map(label),
            "view": if files.view == "history" { "history" } else { "files" },
            "search": label(&files.search),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel_view::{PanelFiles, PanelSelection};

    fn editor(path: &str) -> PanelEditor {
        PanelEditor {
            path: path.to_owned(),
            focused: true,
            side: None,
            saved_version: None,
            unsaved: false,
            unsaved_text: None,
            selection: None,
            cursor_line: Some(8),
            changed_lines: Vec::new(),
        }
    }

    #[test]
    fn an_answer_is_taken_only_for_its_own_session_and_once() {
        let pending = Pending::default();
        let (id, mut reply) = pending.begin("a");
        let view = PanelView {
            open: true,
            tabs: Vec::new(),
            active: None,
            editors: Vec::new(),
            files: None,
        };
        assert!(
            !pending.answer("b", &id, view.clone()),
            "another session's window"
        );
        assert!(reply.try_recv().is_err());
        assert!(pending.answer("a", &id, view.clone()));
        assert_eq!(reply.try_recv().unwrap(), view);
        assert!(
            !pending.answer("a", &id, view),
            "a second answer is refused"
        );
    }

    #[test]
    fn the_answer_carries_unsaved_text_and_the_selection_bounded() {
        let mut open = editor("src/calc.rs");
        open.unsaved = true;
        open.unsaved_text = Some("x".repeat(MAX_TEXT + 5));
        open.selection = Some(PanelSelection {
            start_line: 8,
            end_line: 10,
            text: "a + b".to_owned(),
        });
        open.changed_lines = vec![9];
        let answer = agent_answer(PanelView {
            open: true,
            tabs: vec!["Files".into(), "calc.rs".into(), "Terminal".into()],
            active: Some("calc.rs".into()),
            editors: vec![open],
            files: Some(PanelFiles {
                folder: Some("/w/calc".into()),
                view: "files".into(),
                search: "overflow".into(),
            }),
        });
        let first = &answer["editors"][0];
        assert_eq!(answer["status"], "read");
        assert_eq!(first["path"], "src/calc.rs");
        assert_eq!(
            first["selection"],
            json!({"startLine": 8, "endLine": 10, "text": "a + b"})
        );
        assert_eq!(first["cursorLine"], 8);
        assert_eq!(first["changedLines"], json!([9]));
        assert_eq!(first["unsavedText"].as_str().unwrap().len(), MAX_TEXT);
        assert_eq!(first["unsavedTextCut"], true);
        assert_eq!(answer["files"]["search"], "overflow");
        let notes = answer["notes"].to_string();
        assert!(
            notes.contains("not saved yet") && notes.contains("processes_list"),
            "{notes}"
        );
    }

    #[test]
    fn nothing_open_is_said_plainly() {
        let answer = agent_answer(PanelView {
            open: false,
            tabs: Vec::new(),
            active: None,
            editors: Vec::new(),
            files: None,
        });
        assert_eq!(answer["status"], "nothingOpen");
        assert!(
            answer["notes"][0]
                .as_str()
                .unwrap()
                .starts_with("Nothing is open beside the chat")
        );
    }

    #[test]
    fn the_window_cannot_add_fields() {
        let planted = json!({"open": true, "tabs": [], "active": null, "editors": [], "files": null,
            "terminalOutput": "secret"});
        assert!(serde_json::from_value::<PanelView>(planted).is_err());
        let field = json!({"path": "a", "focused": true, "side": null, "savedVersion": null, "unsaved": false,
            "unsavedText": null, "selection": null, "cursorLine": null, "changedLines": [], "password": "x"});
        assert!(serde_json::from_value::<PanelEditor>(field).is_err());
    }
}
