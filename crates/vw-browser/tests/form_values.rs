//! Native form values and truthful receipts through the production browser driver.
//! Uses the offline Chromium fixture; the same HTML also runs in the native `WebKit` script checks.
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

async fn page() -> Rig {
    let rig = Rig::new().await;
    rig.engine.viewport(1280, 900).await;
    rig.engine.eval(&format!("document.head.innerHTML='';document.body.innerHTML={};window.events=[];for(const kind of ['input','change']) document.addEventListener(kind,e=>window.events.push([e.target.id,kind,e.target.value]));", json!(include_str!("support/value_controls.html")))).await;
    rig
}

async fn act(rig: &Rig, id: &str, mut request: Value) -> Result<Value, vw_browser::ActError> {
    let name = rig
        .engine
        .eval(&format!(
            "document.getElementById({}).labels[0].textContent",
            json!(id)
        ))
        .await;
    let (model, _, _) = rig
        .browser
        .observe(
            SESSION,
            ReadOptions {
                find: name.as_str().map(str::to_owned),
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    request["ref"] = json!(
        model
            .refs
            .iter()
            .find(|r| Some(r.name.as_str()) == name.as_str())
            .unwrap()
            .id
    );
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&request).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
}

#[tokio::test]
async fn sets_form_values_once_with_events_and_current_state() {
    let rig = page().await;
    for (id, value) in [
        ("range", "8"),
        ("color", "#00aa55"),
        ("date", "2026-03-14"),
        ("time", "13:45"),
        ("datetime-local", "2026-03-14T13:45"),
        ("month", "2026-03"),
        ("week", "2026-W11"),
        ("city", "Seattle"),
        ("text-date", "03/14/2026"),
    ] {
        let receipt = act(&rig, id, json!({"action":"type","text":value}))
            .await
            .unwrap();
        assert_eq!(
            receipt["untrustedPageContent"]["target"]["valueNow"], value,
            "{receipt}"
        );
        assert_eq!(
            rig.engine
                .eval(&format!("document.getElementById({}).value", json!(id)))
                .await,
            value
        );
        assert_eq!(
            rig.engine
                .eval(&format!("window.events.filter(e=>e[0]==={})", json!(id)))
                .await,
            json!([[id, "input", value], [id, "change", value]]),
            "one input and change for {id}"
        );
        assert!(
            receipt["change"]["summary"]
                .as_str()
                .unwrap()
                .contains("control changed state"),
            "{receipt}"
        );
    }
}

#[tokio::test]
async fn invalid_native_values_do_not_mutate_or_fire_events() {
    let rig = page().await;
    for (id, invalid) in [
        ("range", "not a number"),
        ("color", "green"),
        ("date", "03/14/2026"),
        ("time", "25:00"),
        ("datetime-local", "yesterday"),
        ("month", "2026-13"),
        ("week", "2026-W99"),
    ] {
        let script = format!("document.getElementById({}).value", json!(id));
        let before = rig.engine.eval(&script).await;
        let error = act(&rig, id, json!({"action":"type","text":invalid}))
            .await
            .unwrap_err();
        assert!(error.text.contains("Use "), "{error:?}");
        assert_eq!(rig.engine.eval(&script).await, before, "{id}");
    }
    assert_eq!(rig.engine.eval("window.events").await, json!([]));
}

#[tokio::test]
async fn colour_click_refuses_the_picker_and_explains_how_to_set_the_value() {
    let rig = page().await;
    let error = act(&rig, "color", json!({"action":"click"}))
        .await
        .unwrap_err();
    assert!(
        error.text.contains("unsupported")
            && error.text.contains("type")
            && error.text.contains("#RRGGBB"),
        "{error:?}"
    );
    assert_eq!(rig.engine.eval("window.events").await, json!([]));
    assert!(
        rig.browser
            .read(SESSION, ReadOptions::default(), false, Actor::Agent)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn select_on_value_inputs_explains_the_supported_action() {
    let rig = page().await;
    for id in ["color", "range", "date", "city"] {
        let error = act(&rig, id, json!({"action":"select","value":"8"}))
            .await
            .unwrap_err();
        assert!(
            error.text.contains("unsupported") && error.text.contains("browser_act type"),
            "{error:?}"
        );
    }
    assert_eq!(rig.engine.eval("window.events").await, json!([]));
}

#[tokio::test]
async fn coordinate_colour_click_is_refused_before_native_input() {
    let rig = page().await;
    rig.engine.eval("document.getElementById('color').style='position:fixed;left:20px;top:20px;width:100px;height:40px'").await;
    let error = act(&rig, "color", json!({"action":"click","x":50,"y":40}))
        .await
        .unwrap_err();
    assert!(
        error.text.contains("unsupported") && error.text.contains("browser_act type"),
        "{error:?}"
    );
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[tokio::test]
async fn unsupported_native_edit_keys_name_the_value_setting_route() {
    let rig = page().await;
    for id in ["color", "date", "time", "datetime-local", "month", "week"] {
        let error = act(&rig, id, json!({"action":"press","key":"ArrowUp"}))
            .await
            .unwrap_err();
        assert!(
            error.text.contains("unsupported") && error.text.contains("browser_act type"),
            "{error:?}"
        );
    }
    assert_eq!(rig.engine.eval("window.events").await, json!([]));
}

#[tokio::test]
async fn range_keys_report_changes_and_respect_bounds() {
    let rig = page().await;
    for (key, value) in [
        ("ArrowRight", "6"),
        ("End", "10"),
        ("Home", "0"),
        ("ArrowUp", "1"),
    ] {
        let receipt = act(&rig, "range", json!({"action":"press","key":key}))
            .await
            .unwrap();
        assert_eq!(
            rig.engine
                .eval("document.getElementById('range').value")
                .await,
            value
        );
        assert!(
            receipt["change"]["summary"]
                .as_str()
                .unwrap()
                .contains("control changed state"),
            "{receipt}"
        );
    }
}

#[tokio::test]
async fn a_changed_range_outside_the_receipt_ref_budget_is_still_reported() {
    let rig = page().await;
    rig.engine.eval("document.querySelector('form').insertAdjacentHTML('afterbegin',Array.from({length:40},(_,i)=>'<label>Before '+i+'<input></label>').join(''));document.getElementById('range').addEventListener('keydown',()=>{document.getElementById('range').value='8'})").await;
    let receipt = act(&rig, "range", json!({"action":"press","key":"x"}))
        .await
        .unwrap();
    assert!(
        receipt["change"]["summary"]
            .as_str()
            .unwrap()
            .contains("control changed state"),
        "{receipt}"
    );
    assert!(
        receipt["untrustedPageContent"]["updatedRefs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["value"] == "8"),
        "{receipt}"
    );
}

#[tokio::test]
async fn a_changed_range_in_a_small_frame_keeps_its_receipt_slot() {
    let rig = page().await;
    rig.engine.eval(r#"document.body.innerHTML=Array.from({length:40},(_,i)=>'<label>Before '+i+'<input></label>').join('');
        new Promise(resolve=>{const frame=document.createElement('iframe');frame.style='position:fixed;left:10px;top:10px;width:300px;height:120px';frame.onload=resolve;
        frame.srcdoc='<label>Nested range<input type="range" min="0" max="10" value="5"></label>';document.body.append(frame)})"#).await;
    let (model, _, _) = rig
        .browser
        .observe(
            SESSION,
            ReadOptions {
                find: Some("Nested range".into()),
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    let reference = &model
        .refs
        .iter()
        .find(|r| r.name == "Nested range")
        .unwrap()
        .id;
    let receipt = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"type","ref":reference,"text":"8"})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    assert!(
        receipt["change"]["summary"]
            .as_str()
            .unwrap()
            .contains("control changed state"),
        "{receipt}"
    );
    assert!(
        receipt["untrustedPageContent"]["updatedRefs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["value"] == "8"),
        "{receipt}"
    );
}

#[tokio::test]
async fn a_key_on_the_focused_range_reports_its_changed_value() {
    let rig = page().await;
    rig.engine.eval("document.querySelector('form').insertAdjacentHTML('afterbegin',Array.from({length:40},(_,i)=>'<label>Before '+i+'<input></label>').join(''));document.getElementById('range').focus()").await;
    rig.browser
        .observe(
            SESSION,
            ReadOptions {
                find: Some("Range".into()),
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    rig.browser
        .observe(
            SESSION,
            ReadOptions {
                find: Some("Before".into()),
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    let receipt = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"press","key":"ArrowRight"})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    assert!(
        receipt["change"]["summary"]
            .as_str()
            .unwrap()
            .contains("control changed state"),
        "{receipt}"
    );
    assert!(
        receipt["untrustedPageContent"]["updatedRefs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["value"] == "6"),
        "{receipt}"
    );
}
