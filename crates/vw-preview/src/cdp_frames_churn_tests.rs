//! A page whose ad and consent frames keep being replaced (each in a process of its own) is still read: a child
//! process that cannot list its frames, again after the recovery, leaves its frames out of an every-frame evaluation
//! instead of failing it. A synchronous scripted transport; no native view is needed.
//! PREDICT (before the change): the second pass found the newly attached child session dead too and the whole
//! evaluation failed with "frame document changed, detached, or could not be evaluated (frame list: …)".
use super::*;
use std::cell::Cell;

/// Answers every call at once. The page's own session lists one main frame; every child session is already gone
/// when asked, and each re-attachment brings a new child session (a frame replaced again).
#[derive(Default)]
struct Churning {
    frames: RefCell<Option<CdpFrames>>,
    attached: Cell<u32>,
}

impl Churning {
    fn attach(&self) {
        let n = self.attached.get() + 1;
        self.attached.set(n);
        let frames = self.frames.borrow().clone();
        if let Some(frames) = frames {
            frames.on_event(
                "",
                "Target.attachedToTarget",
                &json!({"sessionId": format!("ad-{n}"), "targetInfo": {"type": "iframe"}, "waitingForDebugger": false}),
            );
        }
    }
}

impl CdpTransport for Churning {
    fn call(&self, session: &str, method: &str, params: Value, done: CdpDone) {
        let answer = match (session, method) {
            ("", "Page.getFrameTree") => Ok(json!({"frameTree": {"frame": {
                "id": "main", "loaderId": "doc", "url": "https://news.example/article"}}})),
            (_, "Page.getFrameTree") => Err("Session with given id not found.".to_owned()),
            (_, "Target.setAutoAttach") => {
                self.attach();
                Ok(json!({}))
            }
            (_, "Page.createIsolatedWorld") => Ok(json!({"executionContextId": 7})),
            (_, "Runtime.evaluate") if params["expression"] == "location.href" => {
                Ok(json!({"result": {"value": "https://news.example/article"}}))
            }
            (_, "Runtime.evaluate") => Ok(json!({"result": {"value": "read"}})),
            _ => Ok(json!({})),
        };
        done(answer);
    }
}

#[test]
fn a_child_process_that_keeps_going_away_does_not_fail_the_whole_page() {
    let transport = Rc::new(Churning::default());
    let frames = CdpFrames::new(transport.clone());
    *transport.frames.borrow_mut() = Some(frames.clone());
    transport.attach();
    let before = frames.generation();
    let answer = Rc::new(RefCell::new(None));
    let kept = answer.clone();
    frames.evaluate(
        None,
        "1",
        Box::new(move |result| *kept.borrow_mut() = Some(result)),
    );
    let answers = answer
        .borrow_mut()
        .take()
        .expect("the evaluation finished")
        .expect("the page is read");
    assert_eq!(answers.len(), 1, "only the main frame answered");
    assert!(answers[0].main);
    assert_eq!(answers[0].value, "\"read\"");
    assert!(
        frames.generation() > before,
        "a checked click across the lost frames fails closed"
    );
}

#[test]
fn one_frame_of_a_process_that_went_away_still_fails() {
    let transport = Rc::new(Churning::default());
    let frames = CdpFrames::new(transport.clone());
    *transport.frames.borrow_mut() = Some(frames.clone());
    transport.attach();
    let answer = Rc::new(RefCell::new(None));
    let kept = answer.clone();
    frames.evaluate(
        Some("ad-frame"),
        "1",
        Box::new(move |result| *kept.borrow_mut() = Some(result)),
    );
    assert!(
        answer.borrow_mut().take().expect("finished").is_err(),
        "an evaluation of one frame never falls back to another"
    );
}
