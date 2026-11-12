//! Native keyboard future-document fence cleanup with delayed child-session installation.
//! Uses the frame registry's controllable transport; no native browser is required.
use super::*;

#[test]
fn future_document_fence_is_removed_before_completion() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    let (held, done) = completion();
    frames.hold_keyboard("fence".into(), done);
    assert!(held.borrow().is_none());
    assert_eq!(transport.calls.borrow()[0].2["runImmediately"], false);
    transport.answer(Ok(json!({"identifier":"root-fence"})));
    assert_eq!(held.borrow().as_ref(), Some(&Ok(())));
    let (released, done) = completion();
    frames.release_keyboard(done);
    assert!(released.borrow().is_none());
    assert_eq!(
        transport.calls.borrow()[1].1,
        "Page.removeScriptToEvaluateOnNewDocument"
    );
    transport.answer(Ok(json!({})));
    assert_eq!(released.borrow().as_ref(), Some(&Ok(())));
}

#[test]
fn a_child_finishing_installation_during_cleanup_removes_its_own_fence() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    let (_, done) = completion();
    frames.hold_keyboard("fence".into(), done);
    transport.answer(Ok(json!({"identifier":"root-fence"})));
    let (child, done) = completion();
    frames.prepare_keyboard("child".into(), true, done);
    assert_eq!(transport.calls.borrow()[1].2["runImmediately"], true);
    let (released, done) = completion();
    frames.release_keyboard(done);
    transport.answer(Ok(json!({"identifier":"child-fence"})));
    assert!(child.borrow().is_none());
    transport.answer(Ok(json!({})));
    assert_eq!(released.borrow().as_ref(), Some(&Ok(())));
    assert_eq!(transport.calls.borrow()[3].0, "child");
    assert_eq!(transport.calls.borrow()[3].2["identifier"], "child-fence");
    transport.answer(Ok(json!({})));
    assert_eq!(child.borrow().as_ref(), Some(&Ok(())));
}

#[test]
fn fence_install_failure_never_admits_native_input() {
    let transport = Rc::new(Delayed::default());
    let frames = CdpFrames::new(transport.clone());
    let (held, done) = completion();
    frames.hold_keyboard("fence".into(), done);
    transport.answer(Err("guard installation failed".into()));
    assert_eq!(
        held.borrow().as_ref(),
        Some(&Err("guard installation failed".into()))
    );
}
