//! Native inspector stylesheets and isolated-world repair for Windows Browser cosmetics.
//! Depends on the existing CDP channel; the frame registry forwards lifecycle events, including child sessions.
use super::matcher::Matcher;
use crate::CdpTransport;
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::Arc};

type Done = Box<dyn FnOnce(Result<(), String>)>;
type Key = (String, String);
#[derive(Clone)]
struct Frame {
    url: String,
    sheet: Option<String>,
    revision: u64,
}
#[derive(Default)]
struct State {
    matcher: Option<Arc<Matcher>>,
    page: String,
    frames: BTreeMap<Key, Frame>,
    revision: u64,
}
/// Cosmetic rules of one Browser webview; all state and callbacks live on its UI thread.
#[derive(Clone)]
pub struct Cosmetics {
    transport: Rc<dyn CdpTransport>,
    state: Rc<RefCell<State>>,
}
impl Cosmetics {
    /// Binds the native protocol channel. Frame attachment is owned by the existing agent frame registry.
    #[must_use]
    pub fn new(transport: Rc<dyn CdpTransport>) -> Self {
        Self {
            transport,
            state: Rc::default(),
        }
    }

    /// Receives native frame lifecycle events; frame URLs never come from page JavaScript.
    pub fn on_event(&self, session: &str, method: &str, params: &Value) {
        match method {
            "Page.frameNavigated" | "Page.documentOpened" => {
                let frame = &params["frame"];
                let (Some(id), Some(url)) = (frame["id"].as_str(), frame["url"].as_str()) else {
                    return;
                };
                let mut state = self.state.borrow_mut();
                if session.is_empty() && frame["parentId"].is_null() {
                    state.page = url.into();
                }
                state.revision += 1;
                let revision = state.revision;
                state.frames.insert(
                    (session.into(), id.into()),
                    Frame {
                        url: url.into(),
                        sheet: None,
                        revision,
                    },
                );
                drop(state);
                self.apply((session.into(), id.into()), Box::new(log_failure));
            }
            "Page.frameDetached" => {
                if let Some(id) = params["frameId"].as_str() {
                    self.state
                        .borrow_mut()
                        .frames
                        .remove(&(session.into(), id.into()));
                }
            }
            "Target.detachedFromTarget" => {
                if let Some(child) = params["sessionId"].as_str() {
                    self.state
                        .borrow_mut()
                        .frames
                        .retain(|(s, _), _| s != child);
                }
            }
            "Target.attachedToTarget" if params["targetInfo"]["type"] == "iframe" => {
                if let Some(child) = params["sessionId"].as_str() {
                    self.discover(child);
                }
            }
            _ => {}
        }
    }

    /// Reads existing frames, also when a mode is first enabled after the document has loaded.
    pub fn discover(&self, session: &str) {
        let (this, child) = (self.clone(), session.to_owned());
        self.transport.call(
            session,
            "Page.getFrameTree",
            json!({}),
            Box::new(move |result| {
                fn visit(this: &Cosmetics, session: &str, tree: &Value) {
                    this.on_event(
                        session,
                        "Page.frameNavigated",
                        &json!({"frame":tree["frame"]}),
                    );
                    if let Some(children) = tree["childFrames"].as_array() {
                        for child in children {
                            visit(this, session, child);
                        }
                    }
                }
                if let Ok(tree) = result {
                    visit(&this, &child, &tree["frameTree"]);
                }
            }),
        );
    }

    /// Replaces selectors and clears old inline repairs. Completion waits for all known documents.
    pub fn update(&self, matcher: Arc<Matcher>, done: Done) {
        let keys = {
            let mut state = self.state.borrow_mut();
            state.matcher = Some(matcher);
            state.revision += 1;
            let revision = state.revision;
            for frame in state.frames.values_mut() {
                frame.revision = revision;
            }
            state.frames.keys().cloned().collect::<Vec<_>>()
        };
        if keys.is_empty() {
            done(Ok(()));
            return;
        }
        let pending = Rc::new(RefCell::new((keys.len(), None, Some(done))));
        for key in keys {
            let pending = pending.clone();
            self.apply(
                key,
                Box::new(move |result| {
                    let mut p = pending.borrow_mut();
                    if let Err(e) = result {
                        p.1 = Some(e);
                    }
                    p.0 -= 1;
                    if p.0 == 0 {
                        let result = p.1.take().map_or(Ok(()), Err);
                        let done = p.2.take();
                        drop(p);
                        if let Some(done) = done {
                            done(result);
                        }
                    }
                }),
            );
        }
    }

    fn current(&self, key: &Key, revision: u64) -> bool {
        self.state
            .borrow()
            .frames
            .get(key)
            .is_some_and(|f| f.revision == revision)
    }

    fn apply(&self, key: Key, done: Done) {
        let (frame, selectors) = {
            let s = self.state.borrow();
            let (Some(frame), Some(matcher)) = (s.frames.get(&key), &s.matcher) else {
                done(Ok(()));
                return;
            };
            (
                frame.clone(),
                matcher
                    .decide(&frame.url, "document", &s.page, false)
                    .selectors,
            )
        };
        if selectors.is_empty() && frame.sheet.is_none() {
            done(Ok(()));
            return;
        }
        let this = self.clone();
        let session = key.0.clone();
        self.transport.call(
            &session,
            "DOM.enable",
            json!({}),
            Box::new(move |result| {
                if let Err(e) = result {
                    done(Err(e));
                    return;
                }
                let next = this.clone();
                this.transport.call(
                    &key.0.clone(),
                    "CSS.enable",
                    json!({}),
                    Box::new(move |result| {
                        if let Err(e) = result {
                            done(Err(e));
                            return;
                        }
                        next.sheet(key, frame, selectors, done);
                    }),
                );
            }),
        );
    }

    fn sheet(&self, key: Key, frame: Frame, selectors: Vec<String>, done: Done) {
        if !self.current(&key, frame.revision) {
            done(Ok(()));
            return;
        }
        if let Some(sheet) = frame.sheet {
            self.write(key, frame.revision, &sheet, selectors, done);
            return;
        }
        let this = self.clone();
        self.transport.call(
            &key.0.clone(),
            "CSS.createStyleSheet",
            json!({"frameId":key.1}),
            Box::new(move |result| {
                let sheet = match result.and_then(|v| {
                    v["styleSheetId"]
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "missing native stylesheet".into())
                }) {
                    Ok(sheet) => sheet,
                    Err(e) => {
                        done(Err(e));
                        return;
                    }
                };
                if !this.current(&key, frame.revision) {
                    done(Ok(()));
                    return;
                }
                if let Some(f) = this.state.borrow_mut().frames.get_mut(&key) {
                    f.sheet = Some(sheet.clone());
                }
                this.write(key, frame.revision, &sheet, selectors, done);
            }),
        );
    }

    fn write(&self, key: Key, revision: u64, sheet: &str, selectors: Vec<String>, done: Done) {
        let mut css = String::new();
        for selector in &selectors {
            css.push_str(selector);
            css.push_str("{display:none!important}\n");
        }
        let this = self.clone();
        self.transport.call(
            &key.0.clone(),
            "CSS.setStyleSheetText",
            json!({"styleSheetId":sheet,"text":css}),
            Box::new(move |result| {
                if let Err(e) = result {
                    done(Err(e));
                    return;
                }
                if !this.current(&key, revision) {
                    done(Ok(()));
                    return;
                }
                let next = this.clone();
                this.transport.call(
                    &key.0.clone(),
                    "Page.createIsolatedWorld",
                    json!({"frameId":key.1,"worldName":"vibe-work-shields"}),
                    Box::new(move |result| {
                        let context = match result.and_then(|v| {
                            v["executionContextId"]
                                .as_u64()
                                .ok_or_else(|| "missing cosmetic script world".into())
                        }) {
                            Ok(id) => id,
                            Err(e) => {
                                done(Err(e));
                                return;
                            }
                        };
                        if !next.current(&key, revision) {
                            done(Ok(()));
                            return;
                        }
                        let script = include_str!("repair.js")
                            .replace("__SELECTORS__", &json!(selectors).to_string());
                        next.transport.call(
                            &key.0,
                            "Runtime.evaluate",
                            json!({"contextId":context,"expression":script,"returnByValue":true}),
                            Box::new(move |result| {
                                done(result.and_then(|v| {
                                    if v.get("exceptionDetails").is_some() {
                                        Err("cosmetic repair failed".into())
                                    } else {
                                        Ok(())
                                    }
                                }));
                            }),
                        );
                    }),
                );
            }),
        );
    }
}
fn log_failure(result: Result<(), String>) {
    if let Err(error) = result {
        tracing::warn!(%error, "browser cosmetic rules could not update a frame");
    }
}
