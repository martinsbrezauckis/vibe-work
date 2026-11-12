//! Act admission and cleanup against a delayed, failing `DevTools` transport. No native window is needed.
//! Two internal recovery-entry contracts remain here; public control contracts live in the private workspace.
use super::*;
use std::collections::VecDeque;

#[path = "cdp_keyboard_tests.rs"]
mod keyboard;
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
