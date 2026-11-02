//! Act admission and cleanup against a delayed, failing `DevTools` transport. No native window is needed.
//! Called by vw-preview unit tests; callbacks, including errors, arrive only when the test releases them.
use super::*;
use std::collections::VecDeque;

#[path = "cdp_act_recovery_tests.rs"]
mod recovery;

type Pending = (String, String, Value, CdpDone);
#[derive(Default)]
struct Delayed {
    pending: RefCell<VecDeque<Pending>>,
    calls: RefCell<Vec<(String, String, Value)>>,
    closed: std::cell::Cell<usize>,
}
impl CdpTransport for Delayed {
    fn close(&self) -> Result<(), String> {
        self.closed.set(self.closed.get() + 1);
        Ok(())
    }
    fn call(&self, session: &str, method: &str, params: Value, done: CdpDone) {
        self.calls
            .borrow_mut()
            .push((session.into(), method.into(), params.clone()));
        self.pending
            .borrow_mut()
            .push_back((session.into(), method.into(), params, done));
    }
}
impl Delayed {
    fn answer(&self, result: Result<Value, String>) {
        let (_, _, _, done) = self
            .pending
            .borrow_mut()
            .pop_front()
            .expect("pending command");
        done(result);
    }
    fn answer_matching(&self, session: &str, method: &str, result: Result<Value, String>) {
        let index = self
            .pending
            .borrow()
            .iter()
            .position(|(s, m, _, _)| s == session && m == method)
            .unwrap();
        let (_, _, _, done) = self.pending.borrow_mut().remove(index).unwrap();
        done(result);
    }
    fn drain(&self) {
        for _ in 0..100 {
            if self.pending.borrow().is_empty() {
                return;
            }
            self.answer(Ok(json!({})));
        }
        panic!("commands did not settle");
    }
}
type Answer = Rc<RefCell<Option<Result<(), String>>>>;
fn completion() -> (Answer, act::Done) {
    let result = Rc::new(RefCell::new(None));
    let sink = result.clone();
    (result, Box::new(move |r| *sink.borrow_mut() = Some(r)))
}
fn child(frames: &CdpFrames, id: &str) {
    frames.on_event(
        "",
        "Target.attachedToTarget",
        &json!({"sessionId":id,"targetInfo":{"type":"iframe"}}),
    );
}

#[test]
fn admission_waits_for_every_enable_acknowledgement() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    child(&frames, "child");
    transport.drain();
    let (answer, done) = completion();
    frames.start_act(1, done);
    assert!(
        answer.borrow().is_none(),
        "input admitted before enable acknowledgements"
    );
    while transport.pending.borrow().len() > 1 {
        transport.answer(Ok(json!({})));
        assert!(
            answer.borrow().is_none(),
            "not every current session acknowledged"
        );
    }
    transport.drain();
    assert_eq!(answer.borrow().as_ref(), Some(&Ok(())));
}

#[test]
fn a_failed_enable_refuses_admission() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    let (answer, done) = completion();
    frames.start_act(1, done);
    transport.answer(Err("interception unavailable".into()));
    transport.drain();
    assert!(
        answer.borrow().as_ref().is_some_and(Result::is_err),
        "failed enable admitted input"
    );
}

#[test]
fn an_attached_session_gets_the_active_restrictions() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    child(&frames, "late");
    transport.drain();
    for method in [
        "Page.setInterceptFileChooserDialog",
        "Emulation.setFocusEmulationEnabled",
    ] {
        assert!(
            transport
                .calls
                .borrow()
                .iter()
                .any(|(s, m, p)| s == "late" && m == method && p["enabled"] == true),
            "new session lacks {method}"
        );
    }
}

#[test]
fn cleanup_waits_and_retries_a_failed_disable() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    transport.calls.borrow_mut().clear();
    let (answer, done) = completion();
    frames.end_act(1, done);
    assert!(
        answer.borrow().is_none(),
        "cleanup reported before disable acknowledgement"
    );
    transport.answer(Err("transient refusal".into()));
    transport.drain();
    assert_eq!(answer.borrow().as_ref(), Some(&Ok(())));
    assert!(
        transport
            .calls
            .borrow()
            .iter()
            .filter(|(_, m, p)| m == "Page.setInterceptFileChooserDialog" && p["enabled"] == false)
            .count()
            >= 2
    );
}

#[test]
fn a_late_end_does_not_disable_a_newer_act() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    frames.start_act(2, Box::new(|r| r.unwrap()));
    transport.drain();
    transport.calls.borrow_mut().clear();
    frames.end_act(1, Box::new(|r| r.unwrap()));
    assert!(
        transport.calls.borrow().is_empty(),
        "old end disabled the new act"
    );
}

#[test]
fn full_screen_exit_waits_and_retries_while_the_view_is_full_screen() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    let entered = Rc::new(std::cell::Cell::new(true));
    let view = entered.clone();
    let answer = Rc::new(RefCell::new(None));
    let sink = answer.clone();
    frames.exit_full_screen(
        Rc::new(move || Ok(view.get())),
        Box::new(move |r| *sink.borrow_mut() = Some(r)),
    );
    assert!(
        answer.borrow().is_none(),
        "full screen exit was not acknowledged"
    );
    transport.answer(Err("navigation replaced the context".into()));
    assert_eq!(
        answer.borrow_mut().take(),
        Some(Ok(true)),
        "retry while the view remains full screen"
    );
    let view = entered.clone();
    let sink = answer.clone();
    frames.exit_full_screen(
        Rc::new(move || Ok(view.get())),
        Box::new(move |r| *sink.borrow_mut() = Some(r)),
    );
    assert!(answer.borrow().is_none());
    entered.set(false);
    transport.answer(Err("context gone".into()));
    assert_eq!(answer.borrow_mut().take(), Some(Ok(false)));
}

#[test]
fn a_second_disable_failure_closes_the_main_session() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    let (answer, done) = completion();
    frames.end_act(1, done);
    transport.answer(Err("disable refused".into()));
    assert_eq!(transport.closed.get(), 0);
    assert!(answer.borrow().is_none());
    transport.answer(Err("disable refused again".into()));
    assert_eq!(transport.closed.get(), 1);
    assert_eq!(answer.borrow().as_ref(), Some(&Ok(())));
    assert!(frames.act_ready().is_err());
}

#[test]
fn a_failed_child_disable_detaches_that_session_and_checks_the_detach() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    child(&frames, "child");
    transport.drain();
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    let (answer, done) = completion();
    frames.end_act(1, done);
    for _ in 0..2 {
        transport.answer_matching(
            "child",
            "Page.setInterceptFileChooserDialog",
            Err("disable refused".into()),
        );
    }
    assert!(answer.borrow().is_none());
    transport.answer_matching("", "Target.detachFromTarget", Ok(json!({})));
    transport.drain();
    assert_eq!(transport.closed.get(), 0);
    assert_eq!(answer.borrow().as_ref(), Some(&Ok(())));
    assert!(!frames.state.borrow().sessions.contains("child"));
}

#[test]
fn a_frame_attached_during_admission_joins_the_acknowledgement_barrier() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    let (answer, done) = completion();
    frames.start_act(1, done);
    child(&frames, "late");
    for step in 0..3 {
        transport.answer_matching(
            "",
            match step {
                0 => "Page.setInterceptFileChooserDialog",
                1 => "Emulation.setFocusEmulationEnabled",
                _ => "Target.setAutoAttach",
            },
            Ok(json!({})),
        );
    }
    assert!(answer.borrow().is_none());
    assert!(frames.act_ready().is_err());
    transport.drain();
    assert_eq!(answer.borrow().as_ref(), Some(&Ok(())));
    assert!(frames.act_ready().is_ok());
    let calls = transport.calls.borrow();
    assert!(calls.iter().any(|(s, m, p)| s.is_empty()
        && m == "Target.setAutoAttach"
        && p["waitForDebuggerOnStart"] == true));
    let intercepted = calls
        .iter()
        .position(|(s, m, p)| {
            s == "late" && m == "Page.setInterceptFileChooserDialog" && p["enabled"] == true
        })
        .unwrap();
    let resumed = calls
        .iter()
        .position(|(s, m, _)| s == "late" && m == "Runtime.runIfWaitingForDebugger")
        .unwrap();
    assert!(intercepted < resumed);
}

#[test]
fn an_active_windows_new_frame_is_closed_if_interception_fails() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    child(&frames, "late");
    transport.answer_matching(
        "late",
        "Page.setInterceptFileChooserDialog",
        Err("interception refused".into()),
    );
    transport.drain();
    assert_eq!(transport.closed.get(), 1);
    assert!(frames.act_ready().is_err());
    assert!(
        !transport
            .calls
            .borrow()
            .iter()
            .any(|(s, m, _)| s == "late" && m == "Runtime.runIfWaitingForDebugger")
    );
}

#[test]
fn an_obsolete_disable_completion_cannot_retry_or_close_a_new_act() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    frames.end_act(1, Box::new(drop));
    let (answer, done) = completion();
    frames.start_act(2, done);
    assert!(answer.borrow().is_none());
    transport.answer(Err("old disable failed".into()));
    transport.drain();
    assert_eq!(answer.borrow().as_ref(), Some(&Ok(())));
    assert_eq!(transport.closed.get(), 0);
    assert!(frames.act_ready().is_ok());
    frames.abort_act(1, Box::new(|r| r.unwrap()));
    assert_eq!(transport.closed.get(), 0);
}

#[test]
fn a_missing_completion_can_be_torn_down_without_admitting_input() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    let (answer, done) = completion();
    frames.start_act(1, done);
    assert!(answer.borrow().is_none());
    assert!(frames.act_ready().is_err());
    frames.abort_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    assert_eq!(transport.closed.get(), 1);
    assert!(answer.borrow().as_ref().is_some_and(Result::is_err));
    assert!(frames.act_ready().is_err());
    let (retry, done) = completion();
    frames.start_act(2, done);
    assert!(retry.borrow().as_ref().is_some_and(Result::is_err));
}

#[test]
fn simultaneous_end_requests_share_the_confirmed_cleanup() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    let (first, done) = completion();
    frames.end_act(1, done);
    let (second, done) = completion();
    frames.end_act(1, done);
    assert!(first.borrow().is_none() && second.borrow().is_none());
    transport.drain();
    assert_eq!(first.borrow().as_ref(), Some(&Ok(())));
    assert_eq!(second.borrow().as_ref(), Some(&Ok(())));
    assert_eq!(transport.closed.get(), 0);
}

#[test]
fn gesture_input_rechecks_controls_after_the_frame_tree_and_world_are_read() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    let answer = Rc::new(RefCell::new(None));
    let sink = answer.clone();
    frames.evaluate_input(
        None,
        "agentClick()",
        Box::new(move |r| *sink.borrow_mut() = Some(r)),
    );
    transport.answer(Ok(json!({"frameTree":{"frame":{"id":"main", "loaderId":"loader", "url":"https://example.test/"}}})));
    child(&frames, "late");
    transport.answer_matching(
        "",
        "Page.createIsolatedWorld",
        Ok(json!({"executionContextId": 1})),
    );
    transport.answer_matching(
        "",
        "Runtime.evaluate",
        Ok(json!({"result":{"value":"https://example.test/"}})),
    );
    assert!(answer.borrow().as_ref().is_some_and(Result::is_err));
    assert!(
        !transport
            .calls
            .borrow()
            .iter()
            .any(|(_, m, p)| m == "Runtime.evaluate" && p["expression"] == "agentClick()")
    );
    transport.drain();
}
