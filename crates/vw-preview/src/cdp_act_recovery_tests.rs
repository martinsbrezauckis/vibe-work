//! Recovery regressions using the act tests' delayed transport; no native controller or page is needed.
use super::*;

#[test]
fn ordinary_recovery_after_cleanup_keeps_the_view_without_disabling_again() {
    for attachment_fails in [false, true] {
        let transport = Rc::new(Delayed::default());
        let frames = CdpFrames::new(transport.clone());
        child(&frames, "child");
        transport.drain();
        frames.start_act(1, Box::new(|r| r.unwrap()));
        transport.drain();
        frames.end_act(1, Box::new(|r| r.unwrap()));
        transport.drain();
        transport.calls.borrow_mut().clear();
        let answer = Rc::new(RefCell::new(None));
        let sink = answer.clone();
        frames.evaluate(
            None,
            "read()",
            Box::new(move |r| *sink.borrow_mut() = Some(r)),
        );
        transport.answer_matching("", "Page.getFrameTree", Ok(json!({})));
        transport.answer_matching("child", "Page.getFrameTree", Err("frame replaced".into()));
        // Navigation fails both disables if recovery needlessly sends them.
        for _ in 0..20 {
            let method = transport
                .pending
                .borrow()
                .front()
                .map(|(_, m, _, _)| m.clone());
            let Some(method) = method else {
                break;
            };
            transport.answer(
                if method == "Page.setInterceptFileChooserDialog"
                    || attachment_fails && method == "Target.setAutoAttach"
                {
                    Err("navigation replaced the context".into())
                } else {
                    Ok(json!({}))
                },
            );
        }
        assert_eq!(
            transport.closed.get(),
            0,
            "ordinary recovery closed the person's view"
        );
        let calls = transport.calls.borrow();
        assert!(!calls.iter().any(|(_, m, _)| matches!(
            m.as_str(),
            "Page.setInterceptFileChooserDialog" | "Emulation.setFocusEmulationEnabled"
        )));
        assert!(
            calls.iter().any(
                |(_, m, p)| m == "Target.setAutoAttach" && p["waitForDebuggerOnStart"] == false
            )
        );
        assert!(answer.borrow().is_some());
    }
}

#[test]
fn ordinary_recovery_preserves_a_previous_act_failure() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    let (admission, done) = completion();
    frames.start_act(1, done);
    transport.answer(Err("interception unavailable".into()));
    transport.drain();
    transport.calls.borrow_mut().clear();
    let (recovery, done) = completion();
    frames.recover_attachment(done);
    transport.drain();
    assert!(admission.borrow().as_ref().is_some_and(Result::is_err));
    assert_eq!(*recovery.borrow(), *admission.borrow());
    assert!(frames.act_ready().is_err());
    assert!(transport.calls.borrow().is_empty());
}

#[test]
fn recovery_during_cleanup_waits_for_the_existing_disable_round() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    let (cleanup, done) = completion();
    frames.end_act(1, done);
    let calls = transport.calls.borrow().len();
    let (recovery, done) = completion();
    frames.recover_attachment(done);
    assert_eq!(transport.calls.borrow().len(), calls);
    assert!(cleanup.borrow().is_none() && recovery.borrow().is_none());
    transport.answer(Err("transient disable failure".into()));
    assert!(cleanup.borrow().is_none() && recovery.borrow().is_none());
    transport.drain();
    assert_eq!(cleanup.borrow().as_ref(), Some(&Ok(())));
    assert_eq!(recovery.borrow().as_ref(), Some(&Ok(())));
    assert_eq!(transport.closed.get(), 0);
}

#[test]
fn child_tree_recovery_keeps_active_attachment_paused_until_acknowledged() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    child(&frames, "child");
    transport.drain();
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    transport.calls.borrow_mut().clear();
    frames.evaluate_input(None, "agentClick()", Box::new(drop));
    transport.answer_matching("", "Page.getFrameTree", Ok(json!({})));
    transport.answer_matching("child", "Page.getFrameTree", Err("frame replaced".into()));
    // Release any preceding controls, but keep the recovery attachment completion pending.
    while !transport
        .pending
        .borrow()
        .iter()
        .any(|(_, m, _, _)| m == "Target.setAutoAttach")
    {
        transport.answer(Ok(json!({})));
    }
    assert!(
        transport
            .calls
            .borrow()
            .iter()
            .all(|(_, m, p)| m != "Target.setAutoAttach" || p["waitForDebuggerOnStart"] == true),
        "recovery cleared the active pause barrier"
    );
    assert!(
        frames.act_ready().is_err(),
        "recovery admitted input before acknowledgement"
    );
    transport.answer_matching("", "Target.setAutoAttach", Ok(json!({})));
    transport.drain();
    assert!(frames.act_ready().is_ok());
    child(&frames, "replacement");
    assert!(frames.act_ready().is_err());
    transport.drain();
    assert!(frames.act_ready().is_ok());
    assert!(
        transport
            .calls
            .borrow()
            .iter()
            .all(|(_, m, p)| m != "Target.setAutoAttach" || p["waitForDebuggerOnStart"] == true)
    );
}

#[test]
fn a_failed_recovery_acknowledgement_refuses_input() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    child(&frames, "child");
    transport.drain();
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    let answer = Rc::new(RefCell::new(None));
    let sink = answer.clone();
    frames.evaluate_input(
        None,
        "agentClick()",
        Box::new(move |r| *sink.borrow_mut() = Some(r)),
    );
    transport.answer_matching("", "Page.getFrameTree", Ok(json!({})));
    transport.answer_matching("child", "Page.getFrameTree", Err("frame replaced".into()));
    while !transport
        .pending
        .borrow()
        .iter()
        .any(|(_, m, _, _)| m == "Target.setAutoAttach")
    {
        transport.answer(Ok(json!({})));
    }
    transport.answer_matching(
        "",
        "Target.setAutoAttach",
        Err("attachment unavailable".into()),
    );
    transport.drain();
    assert!(answer.borrow().as_ref().is_some_and(Result::is_err));
    assert!(frames.act_ready().is_err());
    assert!(
        !transport
            .calls
            .borrow()
            .iter()
            .any(|(_, m, p)| m == "Runtime.evaluate" && p["expression"] == "agentClick()")
    );
}

#[test]
fn obsolete_failed_detach_leaves_the_new_act_to_confirm_its_controls() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    child(&frames, "child");
    transport.drain();
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    frames.end_act(1, Box::new(drop));
    for _ in 0..2 {
        transport.answer_matching(
            "child",
            "Page.setInterceptFileChooserDialog",
            Err("disable refused".into()),
        );
    }
    let (answer, done) = completion();
    frames.start_act(2, done);
    assert!(frames.act_ready().is_err());
    transport.answer_matching(
        "",
        "Target.detachFromTarget",
        Err("old detach failed".into()),
    );
    assert_eq!(
        transport.closed.get(),
        0,
        "obsolete detach closed the newer act's view"
    );
    assert!(answer.borrow().is_none());
    assert!(frames.act_ready().is_err());
    transport.drain();
    assert_eq!(answer.borrow().as_ref(), Some(&Ok(())));
    assert!(frames.act_ready().is_ok());
    assert!(frames.state.borrow().sessions.contains("child"));
}

#[test]
fn an_active_session_over_the_cap_is_torn_down_instead_of_left_paused() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    frames.start_act(1, Box::new(|r| r.unwrap()));
    transport.drain();
    for i in 0..MAX_SESSIONS {
        child(&frames, &format!("child-{i}"));
        transport.drain();
    }
    assert!(frames.act_ready().is_ok());
    child(&frames, "overflow");
    assert_eq!(
        transport.closed.get(),
        1,
        "overflow left an attached frame paused"
    );
    assert!(frames.act_ready().is_err());
    assert_eq!(frames.state.borrow().sessions.len(), MAX_SESSIONS);
    assert!(!frames.state.borrow().sessions.contains("overflow"));
}

#[test]
fn full_screen_state_read_errors_require_retry_before_and_after_evaluation() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    for after_evaluation in [false, true] {
        let reads = std::cell::Cell::new(0);
        let answer = Rc::new(RefCell::new(None));
        let sink = answer.clone();
        frames.exit_full_screen(
            Rc::new(move || {
                reads.set(reads.get() + 1);
                if after_evaluation && reads.get() == 1 {
                    Ok(true)
                } else {
                    Err("native state unavailable".into())
                }
            }),
            Box::new(move |r| *sink.borrow_mut() = Some(r)),
        );
        if after_evaluation {
            assert!(answer.borrow().is_none());
            transport.answer(Err("old context gone".into()));
        }
        assert_eq!(
            answer.borrow_mut().take(),
            Some(Ok(true)),
            "unconfirmed native state stopped retries"
        );
    }
    frames.exit_full_screen(
        Rc::new(|| Ok(false)),
        Box::new(|r| assert_eq!(r, Ok(false))),
    );
    assert!(transport.pending.borrow().is_empty());
}
