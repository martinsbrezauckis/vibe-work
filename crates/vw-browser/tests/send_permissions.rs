//! Ask send preflight follows label activation and stays inside native focus waits.
//! Uses production scripts in real Chromium, with a fake host focus guard; refused sends dispatch no input.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::{Value, json};
use std::{sync::atomic::Ordering, time::Duration};
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_choice_that_activates_a_post_submitter_signals_before_input() {
    for checked in [true, false] {
        let rig = Rig::new().await;
        rig.engine.eval(&format!("document.body.innerHTML='<form method=post><button role=checkbox aria-checked={} style=\"position:fixed;left:20px;top:20px;width:100px;height:40px\">Send</button></form>'; window.sends=0;document.querySelector('form').onsubmit=e=>{{e.preventDefault();window.sends+=1;}};scrollTo(0,0)", !checked)).await;
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let reference = model
            .refs
            .iter()
            .find(|item| item.name == "Send")
            .unwrap()
            .id
            .clone();
        let action = vw_browser::parse(
            &serde_json::json!({"action":"check","ref":reference,"checked":checked}),
        )
        .unwrap();
        let result = rig
            .browser
            .act_with_sending(SESSION, action, Actor::Agent, &NoNavigation, false)
            .await;
        assert_eq!(result.unwrap_err().text, vw_browser::SEND_REQUIRED);
        assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 0);
        assert_eq!(rig.engine.eval("window.sends").await, serde_json::json!(0));
        let unchanged = vw_browser::parse(
            &serde_json::json!({"action":"check","ref":reference,"checked":!checked}),
        )
        .unwrap();
        assert!(
            rig.browser
                .act_with_sending(SESSION, unchanged, Actor::Agent, &NoNavigation, false)
                .await
                .is_ok()
        );
        assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 0);
        assert_eq!(rig.engine.eval("window.sends").await, serde_json::json!(0));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_focus_wait_keeps_send_detection_in_the_hook_without_dispatching_input() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<form method=get><button style=\"position:fixed;left:20px;top:20px;width:100px;height:40px\">Send</button></form>'; document.querySelector('form').onsubmit=e=>e.preventDefault(); scrollTo(0,0)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = model
        .refs
        .iter()
        .find(|r| r.name == "Send")
        .unwrap()
        .id
        .clone();
    rig.engine.focus_required.store(true, Ordering::Relaxed);
    let browser = rig.browser.clone();
    let task = tokio::spawn(async move {
        let action =
            vw_browser::parse(&serde_json::json!({"action":"click","ref":reference,"x":50,"y":40}))
                .unwrap();
        browser
            .act_with_sending(SESSION, action, Actor::Agent, &NoNavigation, false)
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while rig
            .browser
            .sessions()
            .state(SESSION)
            .pending_native_click
            .is_none()
            && !task.is_finished()
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !task.is_finished(),
        "send checks must remain in the hook until focus settles"
    );
    rig.engine
        .eval("document.querySelector('form').method='post'")
        .await;
    rig.engine.focus_required.store(false, Ordering::Relaxed);
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.unwrap_err().text, vw_browser::SEND_REQUIRED);
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 0);
}

async fn label_fixture(method: &str, contents: &str) -> (Rig, String) {
    let rig = Rig::new().await;
    let html = json!(format!(
        "<style>#target{{position:fixed;left:20px;top:20px;width:200px;height:40px;display:block}} \
         #control{{position:fixed;left:20px;top:100px}}</style><form method={method}>{contents}</form>"
    ));
    rig.engine.eval(&format!(
        "document.body.innerHTML={html}; window.sends=0;window.inputs=0; \
         document.querySelector('form').onsubmit=e=>{{e.preventDefault();window.sends++}}; \
         for(const event of ['click','keydown','keyup']) document.addEventListener(event,()=>window.inputs++); \
         scrollTo(0,0)"
    )).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = model
        .refs
        .iter()
        .find(|item| item.name == "Label action")
        .unwrap()
        .id
        .clone();
    (rig, reference)
}

const ASSOCIATED: &str = "<label id=target role=button tabindex=0 aria-label='Label action' for=control><span>Send via label</span></label><button id=control type=submit>Send</button>";
const WRAPPING: &str = "<label id=target role=button tabindex=0 aria-label='Label action'><span>Send via label</span><button id=control type=submit>Send</button></label>";

#[tokio::test(flavor = "multi_thread")]
async fn post_labels_require_approval_before_scripted_native_and_key_input() {
    let mut failures = Vec::new();
    for contents in [
        ASSOCIATED.to_owned(),
        WRAPPING.to_owned(),
        ASSOCIATED.replace("type=submit", "type=submit hidden"),
        ASSOCIATED.replace(
            "<button id=control type=submit>Send</button>",
            "<input id=control type=submit value=Send>",
        ),
    ] {
        for route in [
            "click",
            "point",
            "Enter",
            "Space",
            "focused Enter",
            "focused Space",
        ] {
            let (rig, reference) = label_fixture("post", &contents).await;
            let action = match route {
                "click" => json!({"action":"click","ref":reference}),
                "point" => json!({"action":"click","ref":reference,"x":40,"y":40}),
                key if key.starts_with("focused ") => {
                    rig.engine
                        .eval("document.querySelector('#target').focus()")
                        .await;
                    json!({"action":"press","key":key.trim_start_matches("focused ")})
                }
                key => json!({"action":"press","ref":reference,"key":key}),
            };
            let result = rig
                .browser
                .act_with_sending(
                    SESSION,
                    vw_browser::parse(&action).unwrap(),
                    Actor::Agent,
                    &NoNavigation,
                    false,
                )
                .await;
            let effects = rig.engine.eval("[window.sends,window.inputs]").await;
            if !result
                .as_ref()
                .is_err_and(|error| error.text == vw_browser::SEND_REQUIRED)
                || effects != json!([0, 0])
                || rig.engine.native_clicks.load(Ordering::Relaxed) != 0
            {
                failures.push(format!(
                    "{contents} / {route}: {result:?}; effects={effects}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_and_non_send_labels_need_no_send_approval() {
    for (method, contents, sends) in [
        ("get", ASSOCIATED.to_owned(), 1),
        ("get", WRAPPING.to_owned(), 1),
        ("post", ASSOCIATED.replace("type=submit", "type=button"), 0),
        (
            "post",
            ASSOCIATED.replace(
                "<button id=control type=submit>Send</button>",
                "<input id=control type=checkbox>",
            ),
            0,
        ),
        (
            "post",
            ASSOCIATED.replace(
                "<button id=control type=submit>Send</button>",
                "<input id=control type=text>",
            ),
            0,
        ),
        ("post", ASSOCIATED.replace("for=control", "for=missing"), 0),
        (
            "post",
            ASSOCIATED.replace("type=submit", "type=submit formmethod=get"),
            1,
        ),
    ] {
        for point in [false, true] {
            let (rig, reference) = label_fixture(method, &contents).await;
            let mut action = json!({"action":"click","ref":reference});
            if point {
                action["x"] = json!(40);
                action["y"] = json!(40);
            }
            let result = rig
                .browser
                .act_with_sending(
                    SESSION,
                    vw_browser::parse(&action).unwrap(),
                    Actor::Agent,
                    &NoNavigation,
                    false,
                )
                .await;
            assert_label_admitted(&rig, point, &result);
            assert_eq!(rig.engine.eval("window.sends").await, json!(sends));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn approved_post_label_sends_once_and_next_activation_needs_approval() {
    for contents in [ASSOCIATED, WRAPPING] {
        for point in [false, true] {
            let (rig, reference) = label_fixture("post", contents).await;
            let mut action = json!({"action":"click","ref":reference});
            if point {
                action["x"] = json!(40);
                action["y"] = json!(40);
            }
            let approved = rig
                .browser
                .act_with_sending(
                    SESSION,
                    vw_browser::parse(&action).unwrap(),
                    Actor::Agent,
                    &NoNavigation,
                    true,
                )
                .await;
            assert_label_admitted(&rig, point, &approved);
            assert_eq!(rig.engine.eval("window.sends").await, json!(1));
            let unapproved = rig
                .browser
                .act_with_sending(
                    SESSION,
                    vw_browser::parse(&action).unwrap(),
                    Actor::Agent,
                    &NoNavigation,
                    false,
                )
                .await;
            assert_eq!(unapproved.unwrap_err().text, vw_browser::SEND_REQUIRED);
            assert_eq!(rig.engine.eval("window.sends").await, json!(1));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn final_native_point_guard_classifies_the_label_descendant() {
    for (method, contents, blocked, unreadable) in [
        ("post", ASSOCIATED.to_owned(), true, false),
        ("post", WRAPPING.to_owned(), true, false),
        ("get", ASSOCIATED.to_owned(), false, false),
        (
            "post",
            ASSOCIATED.replace("type=submit", "type=button"),
            false,
            false,
        ),
        ("post", ASSOCIATED.to_owned(), true, true),
    ] {
        let (rig, reference) = label_fixture(method, &contents).await;
        let page = rig.service.page_accessor(SESSION).unwrap();
        let check = rig.engine.eval(&format!(
            "(()=>{{const env=window[{}],target=document.querySelector('#target'),r=target.getBoundingClientRect(); \
             return {{target:true,document:env.frameDocument,node:env.nodeIdentities.get(target), \
             bounds:{{x:Math.round(r.x),y:Math.round(r.y),width:Math.round(r.width),height:Math.round(r.height)}}, \
             viewport:[innerWidth,innerHeight],x:40,y:30}}}})()", json!(page)
        )).await;
        let script = vw_browser::scripts::point_validation(&json!({
            "page":page,"ref":reference,"checks":[check],"allowSend":false,"receipt":"test-point"
        }));
        if unreadable {
            rig.engine.eval("Object.defineProperty(document.querySelector('#target'),'control',{get(){throw Error('unreadable')}})").await;
        }
        let answer: Value =
            serde_json::from_str(rig.engine.eval(&script).await.as_str().unwrap()).unwrap();
        assert_eq!(answer["ok"], json!(!blocked), "{contents}: {answer}");
        if blocked {
            assert_eq!(answer["reason"], vw_browser::SEND_REQUIRED);
        }
        assert_eq!(
            rig.engine.eval("[window.sends,window.inputs]").await,
            json!([0, 0])
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn label_classification_follows_composed_ancestry_and_fails_closed() {
    let rig = Rig::new().await;
    let source = include_str!("../src/js/sending.js");
    let cases = [
        ("document.querySelector('#target span')", "", true),
        (
            "document.querySelector('#target')",
            "Object.defineProperty(document.querySelector('#target'),'control',{value:undefined})",
            true,
        ),
        (
            "document.querySelector('#target')",
            "Object.defineProperty(document.querySelector('#target'),'control',{get(){throw Error('unreadable')}})",
            true,
        ),
        (
            "document.querySelector('#shadow').shadowRoot.querySelector('span')",
            "const host=document.createElement('span');host.id='shadow';document.querySelector('#target span').replaceWith(host);host.attachShadow({mode:'open'}).innerHTML='<span>Shadow label</span>'",
            true,
        ),
        (
            "document.querySelector('#slotted')",
            "const host=document.createElement('div');host.innerHTML='<span id=slotted>Slotted</span>';document.body.append(host);host.attachShadow({mode:'open'}).innerHTML='<form method=post><label><slot></slot><button>Send</button></label></form>'",
            true,
        ),
        (
            "({localName:'LaBeL',control:document.querySelector('#control')})",
            "",
            true,
        ),
        ("null", "", true),
        (
            "document.querySelector('#target')",
            "document.querySelector('form').method='get';document.querySelector('#control').setAttribute('formmethod','PoSt')",
            true,
        ),
        (
            "document.querySelector('#target')",
            "document.querySelector('form').insertAdjacentHTML('beforeend','<input name=method>')",
            true,
        ),
        (
            "document.querySelector('#target')",
            "document.querySelector('#control').setAttribute('formmethod','')",
            false,
        ),
    ];
    let mut failures = Vec::new();
    for (target, setup, expected) in cases {
        rig.engine
            .eval(&format!(
                "document.body.innerHTML={}",
                json!(format!("<form method=post>{ASSOCIATED}</form>"))
            ))
            .await;
        let result = rig
            .engine
            .eval(&format!(
                "(()=>{{{source}\n{setup};return postsForm({target})}})()"
            ))
            .await;
        if result != json!(expected) {
            failures.push(format!("{target} / {setup}: {result}, expected {expected}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// The native receipt currently regards a label's forwarded trusted click as overlap. These tests
// check admission and actual form effects, retaining that conservative post-input receipt behavior.
fn assert_label_admitted(rig: &Rig, point: bool, result: &Result<Value, vw_browser::ActError>) {
    if point {
        assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 1);
        assert!(
            result.is_ok()
                || result.as_ref().is_err_and(|error| error
                    .text
                    .contains("the page or the user clicked at the same time")),
            "{result:?}"
        );
    } else {
        assert!(result.is_ok(), "{result:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn focused_shadow_label_keys_require_approval_before_input() {
    for key in ["Enter", "Space"] {
        let (rig, _) = label_fixture("post", ASSOCIATED).await;
        rig.engine.eval("const host=document.createElement('div');document.body.append(host);host.attachShadow({mode:'open'}).append(document.querySelector('form'));host.shadowRoot.querySelector('#target').focus()").await;
        rig.browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let action = vw_browser::parse(&json!({"action":"press","key":key})).unwrap();
        let result = rig
            .browser
            .act_with_sending(SESSION, action, Actor::Agent, &NoNavigation, false)
            .await;
        assert_eq!(result.unwrap_err().text, vw_browser::SEND_REQUIRED);
        assert_eq!(
            rig.engine.eval("[window.sends,window.inputs]").await,
            json!([0, 0])
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn label_keys_do_not_implicitly_submit_an_associated_text_field() {
    let contents = ASSOCIATED.replace(
        "<button id=control type=submit>Send</button>",
        "<input id=control type=text>",
    );
    for key in ["Enter", "Space"] {
        let (rig, reference) = label_fixture("post", &contents).await;
        let action =
            vw_browser::parse(&json!({"action":"press","ref":reference,"key":key})).unwrap();
        let result = rig
            .browser
            .act_with_sending(SESSION, action, Actor::Agent, &NoNavigation, false)
            .await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(rig.engine.eval("window.sends").await, json!(0));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn scripted_enter_checks_the_form_method_without_a_submitter() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<form method=post><input aria-label=Message><button formmethod=get>Search</button></form>';window.sends=0;document.querySelector('form').onsubmit=e=>{e.preventDefault();window.sends++};scrollTo(0,0)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = &model
        .refs
        .iter()
        .find(|item| item.name == "Message")
        .unwrap()
        .id;
    let action =
        vw_browser::parse(&json!({"action":"press","ref":reference,"key":"Enter"})).unwrap();
    let result = rig
        .browser
        .act_with_sending(SESSION, action, Actor::Agent, &NoNavigation, false)
        .await;
    assert_eq!(result.unwrap_err().text, vw_browser::SEND_REQUIRED);
    assert_eq!(rig.engine.eval("window.sends").await, json!(0));
}
