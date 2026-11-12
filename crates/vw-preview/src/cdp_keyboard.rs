//! Temporary document-start keyboard fence for native text without an event creation timestamp.
//! Used by `CdpFrames` and its child-session installer; teardown removes exactly the scripts it installed.
use super::{CdpFrames, WORLD};
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

type Done = Box<dyn FnOnce(Result<(), String>)>;
#[derive(Default)]
pub(super) struct Fence {
    source: Option<String>,
    scripts: BTreeMap<String, String>,
}
impl CdpFrames {
    /// Guards future documents until native text has a completed event receipt.
    pub fn hold_keyboard(&self, source: String, done: Done) {
        if self.state.borrow().keyboard.source.is_some() {
            done(Err(
                "native text is already in flight; reopen the browser pane".into(),
            ));
            return;
        }
        self.state.borrow_mut().keyboard.source = Some(source);
        let sessions: Vec<_> = std::iter::once(String::new())
            .chain(
                self.state
                    .borrow()
                    .sessions
                    .iter()
                    .filter(|s| !self.state.borrow().pending_guards.contains(*s))
                    .cloned(),
            )
            .collect();
        let pending = Rc::new(RefCell::new((sessions.len(), None, Some(done))));
        for session in sessions {
            let pending = pending.clone();
            self.prepare_keyboard(
                session,
                false,
                Box::new(move |answer| {
                    let finished = {
                        let mut p = pending.borrow_mut();
                        if let Err(error) = answer {
                            p.1.get_or_insert(error);
                        }
                        p.0 -= 1;
                        (p.0 == 0).then(|| (p.1.take(), p.2.take()))
                    };
                    if let Some((error, Some(done))) = finished {
                        done(error.map_or(Ok(()), Err));
                    }
                }),
            );
        }
    }
    pub(super) fn prepare_keyboard(&self, session: String, immediate: bool, done: Done) {
        let source = self.state.borrow().keyboard.source.clone();
        let Some(source) = source else {
            done(Ok(()));
            return;
        };
        let frames = self.clone();
        self.transport.call(
            &session.clone(),
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source":source,"worldName":WORLD,"runImmediately":immediate}),
            Box::new(move |result| {
                let id = match result.and_then(|r| {
                    r["identifier"]
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "native text guard has no identifier".into())
                }) {
                    Ok(id) => id,
                    Err(error) => {
                        done(Err(error));
                        return;
                    }
                };
                if frames.state.borrow().keyboard.source.as_ref() == Some(&source) {
                    frames
                        .state
                        .borrow_mut()
                        .keyboard
                        .scripts
                        .insert(session, id);
                    done(Ok(()));
                } else {
                    // A child still paused during cleanup may finish installing after the removal snapshot.
                    frames.transport.call(
                        &session,
                        "Page.removeScriptToEvaluateOnNewDocument",
                        json!({"identifier":id}),
                        Box::new(move |answer| done(answer.map(drop))),
                    );
                }
            }),
        );
    }
    /// Removes future-document guards after the completed text receipt, before releasing live guards.
    pub fn release_keyboard(&self, done: Done) {
        let scripts = {
            let mut state = self.state.borrow_mut();
            state.keyboard.source = None;
            std::mem::take(&mut state.keyboard.scripts)
        };
        if scripts.is_empty() {
            done(Ok(()));
            return;
        }
        let pending = Rc::new(RefCell::new((scripts.len(), None, Some(done))));
        for (session, id) in scripts {
            let pending = pending.clone();
            // Detached sessions have no live documents or registered scripts to remove.
            if !session.is_empty() && !self.state.borrow().sessions.contains(&session) {
                let mut p = pending.borrow_mut();
                p.0 -= 1;
                if p.0 == 0 {
                    let error = p.1.take();
                    if let Some(done) = p.2.take() {
                        drop(p);
                        done(error.map_or(Ok(()), Err));
                    }
                }
                continue;
            }
            self.transport.call(
                &session,
                "Page.removeScriptToEvaluateOnNewDocument",
                json!({"identifier":id}),
                Box::new(move |answer| {
                    let finished = {
                        let mut p = pending.borrow_mut();
                        if let Err(error) = answer {
                            p.1.get_or_insert(error);
                        }
                        p.0 -= 1;
                        (p.0 == 0).then(|| (p.1.take(), p.2.take()))
                    };
                    if let Some((error, Some(done))) = finished {
                        done(error.map_or(Ok(()), Err));
                    }
                }),
            );
        }
    }
}
