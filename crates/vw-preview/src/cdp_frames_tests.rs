//! Isolated-world frame evaluation over a fake `DevTools` transport: the page world's (hostile) answers never
//! reach the host, every frame lifecycle change moves the host-owned generation, and stale or failing frames
//! fail closed. The fake keeps a page model per session and records every call.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use std::cell::Cell;

type Change = Box<dyn FnOnce(&Page)>;

#[derive(Default)]
struct Page {
    /// session -> frame tree (CDP `frameTree` object).
    trees: RefCell<BTreeMap<String, Value>>,
    /// (session, frame id) -> address the document reports in its own world.
    href: RefCell<BTreeMap<(String, String), String>>,
    /// What page script made the page world answer (a forged success).
    hostile_world: RefCell<Value>,
    /// What the frame's isolated world computes.
    isolated: RefCell<BTreeMap<String, Value>>,
    /// Frames whose isolated evaluation throws (a page exception text the host must not return).
    throwing: RefCell<BTreeSet<String>>,
    /// context id -> (session, frame id, world name).
    contexts: RefCell<BTreeMap<i64, (String, String, String)>>,
    next_context: Cell<i64>,
    calls: RefCell<Vec<(String, String, Value)>>,
    /// Runs once before the next `Runtime.evaluate` answer (a change while evaluating).
    before_evaluate: RefCell<Option<Change>>,
}

impl CdpTransport for Page {
    fn call(&self, session: &str, method: &str, params: Value, done: CdpDone) {
        self.calls
            .borrow_mut()
            .push((session.into(), method.into(), params.clone()));
        let answer = match method {
            "Page.getFrameTree" => self
                .trees
                .borrow()
                .get(session)
                .map(|tree| json!({"frameTree": tree}))
                .ok_or_else(|| "session detached".to_owned()),
            "Page.createIsolatedWorld" => {
                let id = self.next_context.get() + 1;
                self.next_context.set(id);
                self.contexts.borrow_mut().insert(
                    id,
                    (
                        session.into(),
                        params["frameId"].as_str().unwrap().into(),
                        params["worldName"].as_str().unwrap().into(),
                    ),
                );
                Ok(json!({"executionContextId": id}))
            }
            "Runtime.evaluate" => {
                if let Some(change) = self.before_evaluate.borrow_mut().take() {
                    change(self);
                }
                let Some(context) = params["contextId"].as_i64() else {
                    // The page's own world: whatever page script wants the host to see.
                    return done(Ok(
                        json!({"result": {"type": "object", "value": self.hostile_world.borrow().clone()}}),
                    ));
                };
                let (owner, frame, _) = self.contexts.borrow()[&context].clone();
                assert_eq!(owner, session, "context used in another session");
                if params["expression"] == "location.href" {
                    let href = self.href.borrow()[&(owner, frame)].clone();
                    Ok(json!({"result": {"type": "string", "value": href}}))
                } else if self.throwing.borrow().contains(&frame) {
                    Ok(
                        json!({"result": {"type": "object"}, "exceptionDetails": {"text": "Uncaught secret-value-123"}}),
                    )
                } else {
                    let value = self
                        .isolated
                        .borrow()
                        .get(&frame)
                        .cloned()
                        .unwrap_or(Value::Null);
                    Ok(json!({"result": {"type": "string", "value": value}}))
                }
            }
            _ => Ok(json!({})),
        };
        done(answer);
    }
}

fn frame(id: &str, url: &str, loader: &str, parent: Option<&str>) -> Value {
    json!({"id": id, "url": url, "loaderId": loader, "parentId": parent})
}

fn page() -> (Rc<Page>, CdpFrames) {
    let page = Rc::new(Page::default());
    page.trees.borrow_mut().insert(
        String::new(),
        json!({"frame": frame("MAIN", "https://a.test/", "L1", None),
            "childFrames": [{"frame": frame("CHILD", "https://a.test/child", "L2", Some("MAIN"))}]}),
    );
    for (id, url) in [
        ("MAIN", "https://a.test/"),
        ("CHILD", "https://a.test/child"),
    ] {
        page.href
            .borrow_mut()
            .insert((String::new(), id.into()), url.into());
        page.isolated.borrow_mut().insert(
            id.into(),
            json!(format!("{{\"ok\":false,\"frame\":\"{id}\"}}")),
        );
    }
    *page.hostile_world.borrow_mut() = json!("{\"ok\":true}");
    let frames = CdpFrames::new(page.clone());
    (page, frames)
}

fn run(frames: &CdpFrames, target: Option<&str>, script: &str) -> Result<Vec<FrameAnswer>, String> {
    let out = Rc::new(RefCell::new(None));
    let sink = out.clone();
    frames.evaluate(
        target,
        script,
        Box::new(move |r| *sink.borrow_mut() = Some(r)),
    );
    out.borrow_mut()
        .take()
        .expect("evaluation did not complete")
}

fn registry(answers: &[FrameAnswer]) -> BTreeMap<String, Option<u64>> {
    answers
        .iter()
        .map(|a| (a.id.clone(), a.generation))
        .collect()
}

#[test]
fn a_hostile_page_world_cannot_change_the_isolated_answer() {
    let (page, frames) = page();
    let answers = run(&frames, None, "check()").unwrap();
    assert_eq!(answers.len(), 2);
    for answer in &answers {
        assert!(answer.value.contains("\\\"ok\\\":false"), "{answer:?}");
        assert!(answer.generation.is_some() && answer.origin.is_none());
    }
    assert!(answers[0].main && !answers[1].main);
    // Every evaluation names a context of the private world; none runs in the page's world.
    let calls = page.calls.borrow();
    for (_, method, params) in calls.iter().filter(|(_, m, _)| m == "Runtime.evaluate") {
        let context = params["contextId"].as_i64().expect("page-world evaluation");
        assert_eq!(page.contexts.borrow()[&context].2, WORLD, "{method}");
    }
    assert!(
        calls
            .iter()
            .any(|(_, m, p)| m == "Page.createIsolatedWorld" && p["grantUniveralAccess"] == false)
    );
}

#[test]
fn page_exception_text_never_reaches_the_host() {
    let (page, frames) = page();
    page.throwing.borrow_mut().insert("CHILD".into());
    let error = run(&frames, Some("CHILD"), "check()").unwrap_err();
    assert!(error.starts_with(FAILED), "{error}");
    assert!(!error.contains("secret-value"), "{error}");
    page.throwing.borrow_mut().insert("MAIN".into());
    let error = run(&frames, None, "check()").unwrap_err();
    assert!(error.starts_with(FAILED), "{error}");
    assert!(!error.contains("secret-value"), "{error}");
}

#[test]
fn a_child_that_cannot_be_evaluated_answers_unreadable_in_an_all_frames_evaluation() {
    let (page, frames) = page();
    page.throwing.borrow_mut().insert("CHILD".into());
    let answers = run(&frames, None, "check()").expect("one child failed the whole evaluation");
    let child = answers.iter().find(|a| a.id == "CHILD").unwrap();
    assert_eq!(child.value, crate::UNREADABLE_FRAME);
    assert!(!child.value.contains("secret-value"));
    let main = answers.iter().find(|a| a.id == "MAIN").unwrap();
    assert_eq!(
        main.value,
        json!("{\"ok\":false,\"frame\":\"MAIN\"}").to_string()
    );
}

#[test]
fn a_named_frame_never_falls_back_to_another_frame() {
    let (_, frames) = page();
    let answers = run(&frames, Some("CHILD"), "check()").unwrap();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].id, "CHILD");
    assert_eq!(
        run(&frames, Some("GONE"), "check()").unwrap_err(),
        "frame is stale or detached"
    );
}

#[test]
fn every_frame_lifecycle_change_moves_the_generation() {
    let (page, frames) = page();
    let first = registry(&run(&frames, None, "null").unwrap());
    assert_eq!(
        first,
        registry(&run(&frames, None, "null").unwrap()),
        "an unchanged page kept its generation"
    );
    // A protocol lifecycle event, even of a frame that is gone again by the next read (a transient frame).
    frames.on_event(
        "",
        "Page.frameAttached",
        &json!({"frameId": "T", "parentFrameId": "MAIN"}),
    );
    let second = registry(&run(&frames, None, "null").unwrap());
    assert_ne!(first, second);
    // A replaced document whose event the host has not handled yet: the tree read itself shows it.
    page.trees.borrow_mut().get_mut("").unwrap()["childFrames"][0]["frame"]["loaderId"] =
        json!("L3");
    let third = registry(&run(&frames, None, "null").unwrap());
    assert_ne!(second, third);
    // A frame the host never heard of in an event.
    page.trees.borrow_mut().get_mut("").unwrap()["childFrames"]
        .as_array_mut()
        .unwrap()
        .push(json!({"frame": frame("NEW", "https://a.test/new", "L9", Some("MAIN"))}));
    page.href
        .borrow_mut()
        .insert((String::new(), "NEW".into()), "https://a.test/new".into());
    let fourth = registry(&run(&frames, None, "null").unwrap());
    assert_ne!(third, fourth);
    assert_eq!(fourth.len(), 3);
    for method in CDP_EVENTS {
        let before = frames.generation();
        frames.on_event("", method, &json!({}));
        assert!(
            frames.generation() > before,
            "{method} did not change the generation"
        );
    }
    let before = frames.generation();
    frames.on_event("", "Page.loadEventFired", &json!({}));
    assert_eq!(frames.generation(), before);
}

#[test]
fn a_change_during_evaluation_is_in_the_answers_generation() {
    let (page, frames) = page();
    let before = registry(&run(&frames, None, "null").unwrap());
    let registry_events = frames.clone();
    *page.before_evaluate.borrow_mut() = Some(Box::new(move |_| {
        registry_events.on_event("", "Page.frameNavigated", &json!({}));
    }));
    let after = registry(&run(&frames, None, "null").unwrap());
    assert_ne!(before, after);
}

#[test]
fn a_document_replaced_before_its_world_was_found_has_no_confirmed_address() {
    let (page, frames) = page();
    page.href
        .borrow_mut()
        .insert((String::new(), "CHILD".into()), "https://evil.test/".into());
    let answers = run(&frames, None, "null").unwrap();
    let child = answers.iter().find(|a| a.id == "CHILD").unwrap();
    assert_eq!(child.url, "");
    // A fragment is part of the same document.
    page.href.borrow_mut().insert(
        (String::new(), "CHILD".into()),
        "https://a.test/child#x".into(),
    );
    let answers = run(&frames, None, "null").unwrap();
    assert_eq!(
        answers.iter().find(|a| a.id == "CHILD").unwrap().url,
        "https://a.test/child#x"
    );
}

#[test]
fn frames_in_other_processes_are_evaluated_in_their_own_session() {
    let (page, frames) = page();
    page.trees.borrow_mut().insert(
        "S1".into(),
        json!({"frame": frame("OOP", "https://b.test/", "L5", Some("MAIN"))}),
    );
    page.href
        .borrow_mut()
        .insert(("S1".into(), "OOP".into()), "https://b.test/".into());
    let before = frames.generation();
    frames.on_event(
        "",
        "Target.attachedToTarget",
        &json!({"sessionId": "S1", "targetInfo": {"type": "iframe", "targetId": "OOP"}}),
    );
    assert!(frames.generation() > before);
    {
        let calls = page.calls.borrow();
        assert!(
            calls
                .iter()
                .any(|(s, m, _)| s == "S1" && m == "Page.enable")
        );
        assert!(
            calls
                .iter()
                .any(|(s, m, p)| s == "S1" && m == "Target.setAutoAttach" && p["flatten"] == true)
        );
    }
    // A worker is no frame session.
    frames.on_event(
        "",
        "Target.attachedToTarget",
        &json!({"sessionId": "W1", "targetInfo": {"type": "worker"}}),
    );
    let answers = run(&frames, None, "null").unwrap();
    let ids: Vec<_> = answers.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["MAIN", "CHILD", "OOP"]);
    assert!(!answers[2].main);
    assert!(
        page.calls
            .borrow()
            .iter()
            .any(|(s, m, p)| s == "S1" && m == "Page.createIsolatedWorld" && p["frameId"] == "OOP")
    );
    assert!(!page.calls.borrow().iter().any(|(s, _, _)| s == "W1"));
    // Detached: the frame is gone from the next evaluation and the generation moved.
    let before = frames.generation();
    frames.on_event("", "Target.detachedFromTarget", &json!({"sessionId": "S1"}));
    assert!(frames.generation() > before);
    assert_eq!(run(&frames, None, "null").unwrap().len(), 2);
}

#[test]
fn a_child_session_that_is_gone_is_forgotten_and_its_live_frame_reattached() {
    let (page, frames) = page();
    frames.on_event(
        "",
        "Target.attachedToTarget",
        &json!({"sessionId": "GONE", "targetInfo": {"type": "iframe"}}),
    );
    let before = frames.generation();
    // The frame list fails for the closed session; the read goes on after asking again for every frame in
    // another process, without the dead session.
    let answers = run(&frames, None, "null").unwrap();
    assert_eq!(answers.len(), 2);
    assert!(
        frames.generation() > before,
        "a lost session must invalidate checked input"
    );
    let calls = page.calls.borrow();
    let tail: Vec<_> = calls
        .iter()
        .map(|(s, m, _)| (s.as_str(), m.as_str()))
        .collect();
    assert!(tail.contains(&("", "Target.setAutoAttach")));
    assert_eq!(
        tail.iter()
            .filter(|c| **c == ("GONE", "Page.getFrameTree"))
            .count(),
        1,
        "the dead session was asked again: {tail:?}"
    );
}

#[test]
fn the_main_session_failing_fails_the_evaluation() {
    let (page, frames) = page();
    page.trees.borrow_mut().remove("");
    let error = run(&frames, None, "null").unwrap_err();
    assert_eq!(error, format!("{FAILED} (frame list: session detached)"));
}

#[test]
fn install_enables_events_and_attaches_other_process_frames() {
    let (page, frames) = page();
    let out = Rc::new(Cell::new(false));
    let sink = out.clone();
    frames.install(Box::new(move |r| sink.set(r.is_ok())));
    assert!(out.get());
    let calls = page.calls.borrow();
    let methods: Vec<_> = calls
        .iter()
        .map(|(s, m, _)| (s.as_str(), m.as_str()))
        .collect();
    assert_eq!(methods, [("", "Page.enable"), ("", "Target.setAutoAttach")]);
    assert_eq!(calls[1].2["waitForDebuggerOnStart"], false);
    // No Runtime.enable: pages can notice it, and nothing here needs it.
    assert!(!calls.iter().any(|(_, m, _)| m == "Runtime.enable"));
}

#[test]
fn focus_emulation_reaches_the_page_and_every_frame_session_and_ends_there_too() {
    let (page, frames) = page();
    frames.on_event(
        "",
        "Target.attachedToTarget",
        &json!({"sessionId": "S1", "targetInfo": {"type": "iframe", "targetId": "OOP"}}),
    );
    let focus = |page: &Page| -> Vec<(String, Value)> {
        page.calls
            .borrow()
            .iter()
            .filter(|(_, m, _)| m == "Emulation.setFocusEmulationEnabled")
            .map(|(s, _, p)| (s.clone(), p["enabled"].clone()))
            .collect()
    };
    frames.emulate_focus(true, Box::new(|r| r.unwrap()));
    assert_eq!(
        focus(&page),
        [(String::new(), json!(true)), ("S1".into(), json!(true))]
    );
    page.calls.borrow_mut().clear();
    frames.emulate_focus(false, Box::new(|r| r.unwrap()));
    assert_eq!(
        focus(&page),
        [(String::new(), json!(false)), ("S1".into(), json!(false))]
    );
}
