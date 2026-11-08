//! Declared text-datepicker formats through the production driver, including isolated script worlds.
//! Uses the Chromium fixture and a minimal attached widget; the live Bootstrap widget runs in the `WebKit` test.
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

async fn page(format: Value) -> (Rig, String) {
    let rig = Rig::new().await;
    rig.engine
        .isolate_scripts
        .store(true, std::sync::atomic::Ordering::Relaxed);
    rig.engine.eval(&format!(r#"
        document.body.innerHTML='<label>Date<input id="date" placeholder="mm/dd/yyyy" value="original"></label>';
        window.widgetFormat={format};
        window.jQuery=()=>({{data:()=>window.widgetFormat===null ? null : {{o:{{format:window.widgetFormat}}}}}});
        window.events=[];
        for(const kind of ['input','change','keyup']) document.addEventListener(kind,e=>window.events.push([kind,e.target.value]));
    "#)).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = model
        .refs
        .iter()
        .find(|r| r.name == "Date")
        .unwrap()
        .id
        .clone();
    (rig, reference)
}

async fn type_date(rig: &Rig, reference: &str, text: &str) -> Result<Value, vw_browser::ActError> {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"type","ref":reference,"text":text})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
}

#[tokio::test]
async fn widget_uses_its_declared_numeric_format_and_updates_on_keyup() {
    for (format, expected) in [
        (json!("mm/dd/yyyy"), "03/14/2026"),
        (
            json!({"parts":["dd","mm","yyyy"],"separators":["",".",".",""]}),
            "14.03.2026",
        ),
        (json!("yyyy-m-d"), "2026-3-14"),
    ] {
        let (rig, reference) = page(format).await;
        let receipt = type_date(&rig, &reference, "2026-03-14").await.unwrap();
        assert_eq!(
            receipt["untrustedPageContent"]["target"]["valueNow"], expected,
            "{receipt}"
        );
        assert_eq!(
            rig.engine
                .eval("document.getElementById('date').value")
                .await,
            expected
        );
        assert_eq!(
            rig.engine.eval("window.events").await,
            json!([
                ["input", expected],
                ["change", expected],
                ["keyup", expected]
            ])
        );
    }
}

#[tokio::test]
async fn invalid_widget_dates_keep_the_value_and_name_the_accepted_format() {
    for text in [
        "03142026",
        "14032026",
        "20260314",
        "2026-02-29",
        "02/30/2026",
    ] {
        let (rig, reference) = page(json!("mm/dd/yyyy")).await;
        let error = type_date(&rig, &reference, text).await.unwrap_err();
        assert!(
            error.text.contains("YYYY-MM-DD") && error.text.contains("MM/DD/YYYY"),
            "{error:?}"
        );
        assert_eq!(
            rig.engine
                .eval("document.getElementById('date').value")
                .await,
            "original"
        );
        assert_eq!(rig.engine.eval("window.events").await, json!([]));
    }
}

#[tokio::test]
async fn ordinary_text_is_not_interpreted_as_a_date_and_locale_widget_text_is_retained() {
    for (format, text) in [
        (Value::Null, "20260314"),
        (json!("mm/dd/yyyy"), "03/14/2026"),
    ] {
        let (rig, reference) = page(format).await;
        let receipt = type_date(&rig, &reference, text).await.unwrap();
        assert_eq!(
            receipt["untrustedPageContent"]["target"]["valueNow"], text,
            "{receipt}"
        );
    }
}

#[tokio::test]
async fn unsupported_iso_conversion_does_not_edit_the_widget() {
    let (rig, reference) = page(json!("MM dd, yyyy")).await;
    let error = type_date(&rig, &reference, "2026-03-14").await.unwrap_err();
    assert!(
        error.text.contains("unsupported") && error.text.contains("Use "),
        "{error:?}"
    );
    assert_eq!(
        rig.engine
            .eval("document.getElementById('date').value")
            .await,
        "original"
    );
}

#[tokio::test]
async fn a_shadow_root_datepicker_keeps_its_declared_format() {
    let (rig, _) = page(json!("mm/dd/yyyy")).await;
    rig.engine.eval("const label=document.querySelector('label');const host=document.createElement('div');document.body.append(host);host.attachShadow({mode:'open'}).append(label)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = &model.refs.iter().find(|r| r.name == "Date").unwrap().id;
    let receipt = type_date(&rig, reference, "2026-03-14").await.unwrap();
    assert_eq!(
        receipt["untrustedPageContent"]["target"]["valueNow"], "03/14/2026",
        "{receipt}"
    );
}

#[tokio::test]
async fn an_unrecognized_date_format_still_accepts_literal_text() {
    let (rig, reference) = page(json!("MM dd, yyyy")).await;
    let text = "March 14, 2026";
    let receipt = type_date(&rig, &reference, text).await.unwrap();
    assert_eq!(
        receipt["untrustedPageContent"]["target"]["valueNow"], text,
        "{receipt}"
    );
    assert_eq!(
        rig.engine.eval("window.events").await,
        json!([["input", text], ["change", text], ["keyup", text]])
    );
}

#[tokio::test]
async fn unrelated_datepicker_data_does_not_reinterpret_plain_text() {
    let (rig, reference) = page(Value::Null).await;
    rig.engine
        .eval("window.jQuery=()=>({data:()=>({settings:{dateFormat:'yy-mm-dd'}})})")
        .await;
    let receipt = type_date(&rig, &reference, "2026-03-14").await.unwrap();
    assert_eq!(
        receipt["untrustedPageContent"]["target"]["valueNow"], "2026-03-14",
        "{receipt}"
    );
    assert_eq!(
        rig.engine.eval("window.events").await,
        json!([["input", "2026-03-14"], ["change", "2026-03-14"]])
    );
}

#[tokio::test]
async fn a_committed_widget_date_closes_its_calendar_without_submitting_or_blurring_other_fields() {
    let (rig, reference) = page(json!("mm/dd/yyyy")).await;
    rig.engine
        .eval(
            r"
        const input=document.getElementById('date');
        const calendar=document.createElement('div');
        calendar.id='calendar';calendar.textContent='Calendar';
        input.addEventListener('focus',()=>document.body.append(calendar));
        window.widgetDate=null;window.hidden=0;window.submitted=0;
        window.jQuery=()=>({data:()=>({o:{format:window.widgetFormat},hide(){
            window.hidden++;calendar.remove();
        }})});
        input.addEventListener('keyup',()=>window.widgetDate=input.value);
        document.addEventListener('submit',e=>{e.preventDefault();window.submitted++});
    ",
        )
        .await;
    let receipt = type_date(&rig, &reference, "2026-03-14").await.unwrap();
    assert_eq!(
        receipt["untrustedPageContent"]["target"]["valueNow"],
        "03/14/2026"
    );
    assert_eq!(rig.engine.eval("window.widgetDate").await, "03/14/2026");
    assert_eq!(
        rig.engine
            .eval("Boolean(document.getElementById('calendar'))")
            .await,
        false
    );
    assert_eq!(rig.engine.eval("window.hidden").await, 1);
    assert_eq!(rig.engine.eval("window.submitted").await, 0);
    assert_eq!(rig.engine.eval("document.activeElement.id").await, "date");
}

#[tokio::test]
async fn an_invalid_widget_date_keeps_an_open_calendar_and_does_not_commit() {
    let (rig, reference) = page(json!("mm/dd/yyyy")).await;
    rig.engine.eval("window.hidden=0;window.jQuery=()=>({data:()=>({o:{format:window.widgetFormat},hide(){window.hidden++}})})").await;
    type_date(&rig, &reference, "2026-02-30").await.unwrap_err();
    assert_eq!(rig.engine.eval("window.hidden").await, 0);
    assert_eq!(rig.engine.eval("window.events").await, json!([]));
}

#[tokio::test]
async fn component_and_singleton_calendars_commit_then_close_without_blur_or_submit() {
    for kind in ["bootstrap", "jqueryUi"] {
        let (rig, _) = page(Value::Null).await;
        rig.engine
            .eval(&format!(
                "window.fixtureDateKind={};{}",
                json!(kind),
                include_str!("support/date_widgets.js")
            ))
            .await;
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let reference = &model.refs.iter().find(|r| r.name == "Date").unwrap().id;
        let receipt = type_date(&rig, reference, "2027-04-15").await.unwrap();
        assert_eq!(
            receipt["untrustedPageContent"]["target"]["valueNow"], "04/15/2027",
            "{kind}"
        );
        assert_eq!(
            rig.engine.eval("window.widgetDate").await,
            "04/15/2027",
            "{kind}"
        );
        assert_eq!(
            rig.engine
                .eval("Boolean(document.getElementById('calendar'))")
                .await,
            false,
            "{kind}"
        );
        assert_eq!(
            rig.engine
                .eval("[window.hidden,window.blurs,window.submitted,document.activeElement.id]")
                .await,
            json!([1, 0, 0, "date"]),
            "{kind}"
        );
    }
}

/// Holds the type action's focus back from the page (every focus of the field until `window.released`), as a page
/// without focus does: `WKWebView` whose view is not focused, or a field in a frame that is not the focused one.
/// With `deliver_ms` the page gets that focus later, as such a page does once it is focused.
async fn held_focus_widget(kind: &str, deliver_ms: Option<u64>) -> (Rig, String) {
    let (rig, _) = page(Value::Null).await;
    rig.engine
        .eval(&format!(
            "window.fixtureDateKind={};{}",
            json!(kind),
            include_str!("support/date_widgets.js")
        ))
        .await;
    let deliver = deliver_ms.map_or_else(String::new, |ms| {
        format!("setTimeout(()=>{{window.released=true;date.dispatchEvent(new FocusEvent('focus'));}},{ms});")
    });
    rig.engine
        .eval(&format!(
            "const date=document.getElementById('date');window.order=[];\
             window.addEventListener('focus',e=>{{if(e.target===date&&!window.released){{e.stopImmediatePropagation();{deliver}}}}},true);\
             date.addEventListener('focus',()=>window.order.push('focus'));\
             date.addEventListener('input',()=>window.order.push('input'));"
        ))
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = model
        .refs
        .iter()
        .find(|r| r.name == "Date")
        .unwrap()
        .id
        .clone();
    (rig, reference)
}

/// The date is committed and the calendar closed only after the page has seen the field's focus, so that focus
/// cannot open the calendar over the committed date; the receipt describes that final state.
#[tokio::test]
async fn a_widget_date_is_committed_only_after_the_page_saw_the_fields_focus() {
    for kind in ["bootstrap", "jqueryUi"] {
        let (rig, reference) = held_focus_widget(kind, Some(250)).await;
        let receipt = type_date(&rig, &reference, "2027-04-15").await.unwrap();
        assert_eq!(
            receipt["untrustedPageContent"]["target"]["valueNow"], "04/15/2027",
            "{kind}"
        );
        assert_eq!(
            rig.engine.eval("window.order").await,
            json!(["focus", "input"]),
            "{kind}: the date was typed before the page saw the field's focus"
        );
        assert_eq!(
            rig.engine
                .eval("[Boolean(document.getElementById('calendar')),window.hidden,window.blurs,window.submitted]")
                .await,
            json!([false, 1, 0, 0]),
            "{kind}"
        );
        assert!(
            !receipt.to_string().contains("Calendar"),
            "{kind}: the receipt shows a calendar: {receipt}"
        );
    }
}

/// A page that cannot take focus (its window is not active) never sends it: the date is still typed and closed.
#[tokio::test]
async fn a_page_that_never_sees_the_focus_still_gets_the_date() {
    let (rig, reference) = held_focus_widget("jqueryUi", None).await;
    let receipt = type_date(&rig, &reference, "2027-04-15").await.unwrap();
    assert_eq!(
        receipt["untrustedPageContent"]["target"]["valueNow"],
        "04/15/2027"
    );
    assert_eq!(rig.engine.eval("window.order").await, json!(["input"]));
    assert_eq!(
        rig.engine
            .eval("[Boolean(document.getElementById('calendar')),window.blurs,window.submitted]")
            .await,
        json!([false, 0, 0])
    );
}
