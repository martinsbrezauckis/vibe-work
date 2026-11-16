//! Which built-in inputs can send a POST form, with and without send authority from the core.
//! Uses production scripts in real Chromium; a document submit listener records every submission.
//!
//! PREDICT: guard tests (no red on the base). Drag acts dispatch only synthetic HTML drag events, so a drop on a POST
//! submitter never activates it, with or without send authority. Text entry and insertion carry a line break as
//! text, never as an Enter key, so neither submits. `type` with `submit` presses a separate checked Enter; without
//! send authority it types the text, stops before that Enter and sends nothing. Keys (Enter in a field,
//! Enter or Space on a submitter) are covered in `send_permissions.rs`. A form fill whose choice field is a POST
//! submitter sends only with send authority and otherwise stops at that field. The core decides send authority.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

/// A POST form with a text field, a submit button and a draggable item; returns the refs by name.
async fn post_form() -> (Rig, std::collections::BTreeMap<String, String>) {
    let rig = Rig::new().await;
    rig.engine
        .eval(&format!(
            "document.body.innerHTML={};window.submissions=0;window.clicks=0; \
         document.addEventListener('submit',e=>{{e.preventDefault();window.submissions++}}); \
         document.addEventListener('click',()=>window.clicks++);scrollTo(0,0)",
            json!(
                "<form method=post action=/post><input aria-label='Message'>\
               <button draggable=true>Send POST</button></form>\
               <div draggable=true role=button aria-label='Item'>Item</div>"
            )
        ))
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let refs = model
        .refs
        .iter()
        .filter(|item| ["Message", "Send POST", "Item"].contains(&item.name.as_str()))
        .map(|item| (item.name.clone(), item.id.clone()))
        .collect();
    (rig, refs)
}

async fn act(rig: &Rig, args: &Value, allow: bool) -> Result<Value, vw_browser::ActError> {
    rig.browser
        .act_with_sending(
            SESSION,
            vw_browser::parse(args).unwrap(),
            Actor::Agent,
            &NoNavigation,
            allow,
        )
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drag_onto_a_post_submitter_never_activates_it() {
    let mut failures = Vec::new();
    for allow in [false, true] {
        for (from, to) in [
            ("Item", "Send POST"),
            ("Send POST", "Send POST"),
            ("Message", "Send POST"),
        ] {
            let (rig, refs) = post_form().await;
            let result = act(
                &rig,
                &json!({"action":"drag","ref":refs[from],"to":refs[to]}),
                allow,
            )
            .await;
            let effects = rig.engine.eval("[window.submissions,window.clicks]").await;
            if effects != json!([0, 0]) {
                failures.push(format!(
                    "allow={allow} {from}->{to}: {result:?}; effects={effects}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn typed_or_inserted_line_breaks_do_not_submit_a_post_form() {
    let mut failures = Vec::new();
    for args in [
        json!({"action":"type","text":"Hello\n"}),
        json!({"action":"type","text":"Hello\r\n","submit":false}),
        json!({"action":"insert_text","text":"a\nb"}),
    ] {
        let (rig, refs) = post_form().await;
        let mut args = args;
        args["ref"] = json!(refs["Message"]);
        let result = act(&rig, &args, true).await;
        let sent = rig.engine.eval("window.submissions").await;
        if sent != json!(0) {
            failures.push(format!("{args}: {result:?}; submissions={sent}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn type_and_submit_sends_only_with_send_authority() {
    let (rig, refs) = post_form().await;
    let args = json!({"action":"type","ref":refs["Message"],"text":"Hello","submit":true});
    let denied = act(&rig, &args, false).await;
    // The text was typed before the checked Enter, so the refusal is reported as after-input, never replayable.
    assert_eq!(rig.engine.eval("window.submissions").await, json!(0));
    let text = denied.unwrap_err().text;
    assert!(
        text.starts_with(vw_browser::POST_INPUT_FAILED) && text.contains(vw_browser::SEND_REQUIRED),
        "{text}"
    );
    let (rig, refs) = post_form().await;
    let args = json!({"action":"type","ref":refs["Message"],"text":"Hello","submit":true});
    let allowed = act(&rig, &args, true).await;
    assert!(allowed.is_ok(), "{allowed:?}");
    assert_eq!(rig.engine.eval("window.submissions").await, json!(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_form_fill_whose_choice_field_is_a_post_submitter_sends_only_with_send_authority() {
    for allow in [false, true] {
        let rig = Rig::new().await;
        rig.engine.eval(&format!(
            "document.body.innerHTML={};window.submissions=0; \
             document.addEventListener('submit',e=>{{e.preventDefault();window.submissions++}});scrollTo(0,0)",
            json!("<form method=post action=/post><input aria-label='Message'>\
                   <button role=checkbox aria-checked=false aria-label='Agree'>Agree</button></form>")
        )).await;
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let reference = |name: &str| {
            model
                .refs
                .iter()
                .find(|item| item.name == name)
                .unwrap()
                .id
                .clone()
        };
        let args = json!({"action":"form","fields":[
            {"action":"type","ref":reference("Message"),"text":"Hi"},
            {"action":"check","ref":reference("Agree"),"checked":true}]});
        let receipt = rig
            .browser
            .fill_form(SESSION, &args, Actor::Agent, &NoNavigation, allow)
            .await
            .unwrap();
        let sent = rig.engine.eval("window.submissions").await;
        if allow {
            assert_eq!(sent, json!(1), "{receipt}");
        } else {
            assert_eq!(sent, json!(0), "{receipt}");
            assert_eq!(receipt["completed"], 1, "{receipt}");
            assert_eq!(receipt["error"], vw_browser::SEND_REQUIRED, "{receipt}");
        }
    }
}
