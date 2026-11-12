//! Recovery regressions using the act tests' delayed transport; no native controller or page is needed.
use super::*;

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
