//! Closed shadow roots for a protected-value scan over the Chromium `DevTools` protocol (`WebView2`).
//! A script world cannot enter a closed shadow root, and one the HTML parser attaches (declarative shadow DOM,
//! `<template shadowrootmode="closed">`) passes no page hook either. The protocol's DOM search
//! (`DOM.performSearch` with a CSS query, author shadow trees included, user-agent ones not) finds every form
//! field and editable element of the frame's process, closed trees and same-process child documents included;
//! each is resolved into the frame's private world, and the closed shadow roots they sit in are handed to the scan
//! as its `closedRoots`, so the scan reads those roots exactly as it reads open ones. A closed root without a
//! field holds nothing the scan counts.
//!
//! Only a script with a line exactly [`CLOSED_ROOTS_LINE`] gets them (vw-browser's protected scan). An agent's
//! own script text is always embedded as a JSON string, which cannot hold a bare line break, so it cannot make
//! that line. Every protocol failure fails the frame's evaluation (the admission then refuses).
//! The DOM steps of one channel run one at a time: `DOM.getDocument` renews the protocol's node ids, so a second
//! search must not start between another one's search and its node resolution.
//!
//! Depends on: `serde_json`, [`CdpTransport`]. Called by `cdp_frames::evaluate_frame` on the adapter's UI thread.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::{Value, json};

use super::CdpTransport;

/// The line that asks the engine for the frame's closed shadow roots as the script's `closedRoots`.
pub const CLOSED_ROOTS_LINE: &str = "//vibe-work:closed-roots";
/// More fields than this in one process fail the evaluation (the admission refuses).
const MAX_FIELDS: u64 = 2000;
/// Every element the scan can count as a field: the shared privateField rule looks only at these and editing hosts.
const FIELDS: &str = "input, textarea, select, [contenteditable]";

/// `(statements before, expression after)` when `script` asks for closed roots.
pub(crate) fn split(script: &str) -> Option<(&str, &str)> {
    let mut offset = 0;
    for line in script.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == CLOSED_ROOTS_LINE {
            return Some((&script[..offset], &script[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

/// The function the scan runs as: the statements first, then the scan expression's value, with the closed roots of
/// the found fields in scope.
fn declaration(before: &str, expression: &str) -> String {
    format!(
        "function (...fields) {{\n\
         const closedRoots = Array.from(new Set(fields.map((field) => field.getRootNode())\
         .filter((root) => root instanceof ShadowRoot && root.mode === \"closed\")));\n\
         {before}\nreturn (\n{expression}\n);\n}}"
    )
}

/// Options of the final call (`userGesture`, `awaitPromise`).
#[derive(Clone, Copy)]
pub(crate) struct Call {
    pub(crate) gesture: bool,
    pub(crate) await_promise: bool,
}

type Release = Box<dyn FnOnce()>;
type Job = Box<dyn FnOnce(Release, u64)>;

/// One channel's DOM searches, one at a time.
#[derive(Default)]
pub(crate) struct Queue {
    busy: bool,
    waiting: VecDeque<Job>,
    /// Numbers the object group of each search's resolved fields.
    next: u64,
}

fn enqueue(queue: &Rc<RefCell<Queue>>, job: Job) {
    let start = {
        let mut state = queue.borrow_mut();
        if state.busy {
            state.waiting.push_back(job);
            None
        } else {
            state.busy = true;
            Some(job)
        }
    };
    if let Some(job) = start {
        run(queue, job);
    }
}

fn run(queue: &Rc<RefCell<Queue>>, job: Job) {
    let number = {
        let mut state = queue.borrow_mut();
        state.next += 1;
        state.next
    };
    let again = queue.clone();
    job(Box::new(move || next(&again)), number);
}

fn next(queue: &Rc<RefCell<Queue>>) {
    let job = {
        let mut state = queue.borrow_mut();
        let job = state.waiting.pop_front();
        if job.is_none() {
            state.busy = false;
        }
        job
    };
    if let Some(job) = job {
        run(queue, job);
    }
}

/// Runs the scan in `context` with the closed roots of every field of `session`'s process; `done` gets the protocol
/// answer of `Runtime.callFunctionOn` (or the step's error).
pub(crate) fn evaluate(
    transport: &Rc<dyn CdpTransport>,
    queue: &Rc<RefCell<Queue>>,
    session: &str,
    context: i64,
    script: &str,
    call: Call,
    done: Box<dyn FnOnce(Result<Value, String>)>,
) {
    let Some((before, expression)) = split(script) else {
        done(Err("closed roots were not asked for".into()));
        return;
    };
    let function = declaration(before, expression);
    let transport = transport.clone();
    let session = session.to_owned();
    enqueue(
        queue,
        Box::new(move |release, number| {
            let group = format!("vibe-work-closed-roots-{number}");
            let search = Search {
                transport: transport.clone(),
                session: session.clone(),
                context,
                group: group.clone(),
            };
            search.fields(Box::new(move |objects| {
                release();
                let objects = match objects {
                    Ok(objects) => objects,
                    Err(error) => {
                        free(&transport, &session, &group);
                        done(Err(error));
                        return;
                    }
                };
                let arguments: Vec<Value> =
                    objects.iter().map(|id| json!({"objectId": id})).collect();
                let (after, after_session) = (transport.clone(), session.clone());
                transport.call(
                    &session,
                    "Runtime.callFunctionOn",
                    json!({"functionDeclaration": function, "executionContextId": context,
                        "arguments": arguments, "returnByValue": true, "silent": true,
                        "userGesture": call.gesture, "awaitPromise": call.await_promise}),
                    Box::new(move |answer| {
                        free(&after, &after_session, &group);
                        done(answer);
                    }),
                );
            }));
        }),
    );
}

type Found = Box<dyn FnOnce(Result<Vec<String>, String>)>;

struct Search {
    transport: Rc<dyn CdpTransport>,
    session: String,
    context: i64,
    group: String,
}

impl Search {
    /// Document, search, its results, then every field resolved into the frame's world.
    fn fields(self, found: Found) {
        let transport = self.transport.clone();
        let session = self.session.clone();
        transport.call(
            &session,
            "DOM.getDocument",
            json!({"depth": 0}),
            Box::new(move |document| {
                if let Err(error) = document {
                    found(Err(format!("closed roots: {error}")));
                    return;
                }
                let follow = self.transport.clone();
                follow.call(
                    &self.session.clone(),
                    "DOM.performSearch",
                    json!({"query": FIELDS, "includeUserAgentShadowDOM": false}),
                    Box::new(move |search| self.results(search, found)),
                );
            }),
        );
    }

    fn results(self, search: Result<Value, String>, found: Found) {
        let search = search.ok().and_then(|search| {
            Some((
                search["searchId"].as_str()?.to_owned(),
                search["resultCount"].as_u64()?,
            ))
        });
        let Some((id, count)) = search else {
            found(Err("closed roots: search".into()));
            return;
        };
        if count == 0 || count > MAX_FIELDS {
            self.discard(&id);
            found(if count == 0 {
                Ok(Vec::new())
            } else {
                Err("closed roots: too many fields".into())
            });
            return;
        }
        let transport = self.transport.clone();
        transport.call(
            &self.session.clone(),
            "DOM.getSearchResults",
            json!({"searchId": id, "fromIndex": 0, "toIndex": count}),
            Box::new(move |results| {
                self.discard(&id);
                let nodes: Option<Vec<i64>> = results.ok().and_then(|results| {
                    results["nodeIds"]
                        .as_array()?
                        .iter()
                        .map(Value::as_i64)
                        .collect()
                });
                match nodes {
                    Some(nodes) if !nodes.is_empty() => self.resolve(nodes, found),
                    _ => found(Err("closed roots: search results".into())),
                }
            }),
        );
    }

    fn discard(&self, id: &str) {
        self.transport.call(
            &self.session,
            "DOM.discardSearchResults",
            json!({"searchId": id}),
            Box::new(|_| {}),
        );
    }

    fn resolve(self, nodes: Vec<i64>, found: Found) {
        let state = Rc::new(RefCell::new(Resolving {
            objects: vec![None; nodes.len()],
            left: nodes.len(),
            failed: false,
            found: Some(found),
        }));
        for (index, node) in nodes.into_iter().enumerate() {
            let state = state.clone();
            self.transport.call(
                &self.session,
                "DOM.resolveNode",
                json!({"nodeId": node, "executionContextId": self.context, "objectGroup": self.group}),
                Box::new(move |answer| {
                    let id = answer
                        .ok()
                        .and_then(|a| a["object"]["objectId"].as_str().map(str::to_owned));
                    let ready = {
                        let mut state = state.borrow_mut();
                        match id {
                            Some(id) => state.objects[index] = Some(id),
                            None => state.failed = true,
                        }
                        state.left -= 1;
                        (state.left == 0).then(|| {
                            let objects = std::mem::take(&mut state.objects);
                            let result = if state.failed {
                                Err("closed roots: field".to_owned())
                            } else {
                                Ok(objects.into_iter().flatten().collect())
                            };
                            (state.found.take(), result)
                        })
                    };
                    if let Some((Some(found), result)) = ready {
                        found(result);
                    }
                }),
            );
        }
    }
}

struct Resolving {
    objects: Vec<Option<String>>,
    left: usize,
    failed: bool,
    found: Option<Found>,
}

fn free(transport: &Rc<dyn CdpTransport>, session: &str, group: &str) {
    transport.call(
        session,
        "Runtime.releaseObjectGroup",
        json!({"objectGroup": group}),
        Box::new(|_| {}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_exact_line_asks_for_closed_roots() {
        let script = format!("init();\n{CLOSED_ROOTS_LINE}\n(() => 1)()");
        assert_eq!(split(&script), Some(("init();\n", "(() => 1)()")));
        assert_eq!(split("(() => 1)()"), None);
        // An agent's script arrives JSON-encoded: no bare line break, so never the exact line.
        let embedded = serde_json::json!(format!("\n{CLOSED_ROOTS_LINE}\n")).to_string();
        assert_eq!(split(&format!("run({embedded})")), None);
        assert_eq!(split(&format!("x {CLOSED_ROOTS_LINE}\n1")), None);
    }

    #[test]
    fn searches_on_one_channel_run_one_at_a_time() {
        let queue = Rc::new(RefCell::new(Queue::default()));
        let log = Rc::new(RefCell::new(Vec::new()));
        let held = Rc::new(RefCell::new(None::<Release>));
        for name in ["first", "second"] {
            let (log, held) = (log.clone(), held.clone());
            enqueue(
                &queue,
                Box::new(move |release, number| {
                    log.borrow_mut().push(format!("{name} {number}"));
                    *held.borrow_mut() = Some(release);
                }),
            );
        }
        assert_eq!(*log.borrow(), vec!["first 1"], "the second waits");
        let release = held.borrow_mut().take().unwrap();
        release();
        assert_eq!(*log.borrow(), vec!["first 1", "second 2"]);
        let release = held.borrow_mut().take().unwrap();
        release();
        assert!(!queue.borrow().busy);
    }
}
