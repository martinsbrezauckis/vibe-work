//! Protocol-boundary tests: cosmetics are host-owned, update existing documents and clear on Off.
use super::{cosmetic::Cosmetics, matcher::Matcher};
use crate::{CdpDone, CdpTransport};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[derive(Default)]
struct Wire {
    calls: RefCell<Vec<(String, Value)>>,
}
impl CdpTransport for Wire {
    fn call(&self, _: &str, method: &str, params: Value, done: CdpDone) {
        self.calls.borrow_mut().push((method.into(), params));
        done(Ok(match method {
            "CSS.createStyleSheet" => json!({"styleSheetId":"sheet"}),
            "Page.createIsolatedWorld" => json!({"executionContextId":7}),
            _ => json!({}),
        }));
    }
}
#[test]
fn native_sheet_and_isolated_repair_replace_and_clear() {
    let wire = Rc::new(Wire::default());
    let cosmetics = Cosmetics::new(wire.clone());
    cosmetics.on_event(
        "",
        "Page.frameNavigated",
        &json!({"frame":{"id":"main","url":"https://news.example/"}}),
    );
    cosmetics.update(
        Arc::new(Matcher::new(&super::compile(super::Mode::Balanced, None).unwrap()).unwrap()),
        Box::new(|r| r.unwrap()),
    );
    assert!(wire.calls.borrow().iter().any(|(m, p)| {
        m == "CSS.setStyleSheetText"
            && p["text"]
                .as_str()
                .unwrap()
                .contains("display:none!important")
    }));
    assert!(
        wire.calls
            .borrow()
            .iter()
            .any(|(m, p)| m == "Runtime.evaluate" && p["contextId"] == 7)
    );
    wire.calls.borrow_mut().clear();
    cosmetics.update(
        Arc::new(Matcher::new("[]").unwrap()),
        Box::new(|r| r.unwrap()),
    );
    assert!(
        wire.calls
            .borrow()
            .iter()
            .any(|(m, p)| m == "CSS.setStyleSheetText" && p["text"] == "")
    );
}
