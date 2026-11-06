//! Field discovery and compact state receipts through the production browser loop.
//! Uses the Chromium test engine; no public form or consent is submitted.
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

async fn page(html: &str) -> Rig {
    let rig = Rig::new().await;
    rig.engine.viewport(1000, 800).await;
    rig.engine.eval(&format!("document.head.innerHTML=''; document.body.style=''; document.body.innerHTML={}; scrollTo(0,0)", json!(html))).await;
    rig
}

async fn read(rig: &Rig, query: Option<&str>) -> Value {
    rig.browser
        .read(
            SESSION,
            ReadOptions {
                find: query.map(str::to_owned),
                ..ReadOptions::default()
            },
            false,
            Actor::Agent,
        )
        .await
        .unwrap()
        .0
}

async fn act(rig: &Rig, request: Value) -> Value {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&request).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn unnamed_selects_are_found_by_option_text_and_current_value() {
    let rig = page("<div><h2>Appointment</h2><select id=month><option value=10 selected>November</option><option value=2>March</option></select><select id=year><option>2026</option><option>2027</option></select></div>").await;
    for query in ["November", "10", "March", "2027", "Appointment"] {
        let answer = read(&rig, Some(query)).await;
        let refs = answer["untrustedPageContent"]["refs"].as_array().unwrap();
        assert!(
            refs.iter().any(|r| r["tag"] == "select"),
            "{query}: {answer}"
        );
    }
    let answer = read(&rig, Some("March")).await;
    let reference = &answer["untrustedPageContent"]["refs"][0]["ref"];
    let receipt = act(
        &rig,
        json!({"action":"select","ref":reference,"value":"March"}),
    )
    .await;
    assert_eq!(
        receipt["untrustedPageContent"]["target"]["valueNow"],
        "March"
    );
    assert_eq!(
        rig.engine
            .eval("document.querySelector('#month').value")
            .await,
        "2"
    );
}

#[tokio::test]
async fn option_search_reaches_beyond_the_preview_without_reading_hidden_choices() {
    let rig = page("<select id=years></select>").await;
    rig.engine.eval("years.innerHTML=Array.from({length:120},(_,i)=>'<option>'+ (1950+i) +'</option>').join('')+'<option hidden>Hidden option</option>'").await;
    let answer = read(&rig, Some("2069")).await;
    assert_eq!(
        answer["untrustedPageContent"]["refs"][0]["tag"], "select",
        "{answer}"
    );
    let hidden = read(&rig, Some("Hidden option")).await;
    assert_eq!(
        hidden["untrustedPageContent"]["refs"],
        json!([]),
        "{hidden}"
    );
}

#[tokio::test]
async fn receipt_reports_existing_controls_that_changed_state() {
    let rig = page("<button id=change>Change choices</button><button id=save disabled>Save and close</button><div role=switch aria-checked=true id=purpose>Personalisation</div>").await;
    rig.engine
        .eval(
            "change.onclick=()=>{save.disabled=false;purpose.setAttribute('aria-checked','false')}",
        )
        .await;
    let before = read(&rig, None).await;
    let reference = &before["untrustedPageContent"]["refs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "Change choices")
        .unwrap()["ref"];
    let receipt = act(&rig, json!({"action":"click","ref":reference})).await;
    let updates = receipt["untrustedPageContent"]["updatedRefs"]
        .as_array()
        .expect("receipt needs current control states");
    assert!(
        updates
            .iter()
            .any(|r| r["name"] == "Save and close" && r["disabled"] != true),
        "{receipt}"
    );
    assert!(
        updates
            .iter()
            .any(|r| r["name"] == "Personalisation" && r["checked"] == false),
        "{receipt}"
    );
    assert!(receipt.to_string().len() <= vw_browser::observe::ACT_BUDGET);
    assert!(
        serde_json::to_string_pretty(&receipt).unwrap().len() <= vw_browser::observe::PRETTY_BUDGET
    );
}

#[tokio::test]
async fn invalid_native_date_is_refused_without_erasing_the_existing_value() {
    let rig = page("<label>Appointment date<input type=date value=2026-11-04></label><button type=button>Other control</button>").await;
    let answer = read(&rig, Some("Appointment date")).await;
    let reference = &answer["untrustedPageContent"]["refs"][0]["ref"];
    let error = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"type","ref":reference,"text":"11/04/2026"}))
                .unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap_err();
    assert_eq!(
        rig.engine
            .eval("document.querySelector('input').value")
            .await,
        "2026-11-04",
        "invalid format must not erase a valid date"
    );
    assert!(error.text.contains("YYYY-MM-DD"), "{error:?}");
    let accepted = act(
        &rig,
        json!({"action":"type","ref":reference,"text":"2026-03-14"}),
    )
    .await;
    assert_eq!(
        accepted["untrustedPageContent"]["target"]["valueNow"],
        "2026-03-14"
    );
    act(&rig, json!({"action":"press","ref":reference,"key":"Tab"})).await;
    assert_eq!(
        rig.engine.eval("document.activeElement.tagName").await,
        "BUTTON"
    );
    let fresh = read(&rig, Some("Appointment date")).await;
    assert_eq!(
        fresh["untrustedPageContent"]["refs"][0]["value"],
        "2026-03-14"
    );
}
