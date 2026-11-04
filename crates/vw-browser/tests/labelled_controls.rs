//! Labelled form discovery and read budgeting with a saved public form and small label fixtures.
//! Uses the headless browser driver; all fixtures are offline and no form is submitted.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use std::fmt::Write as _;
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
    rig.engine.viewport(1280, 800).await;
    rig.engine.eval(&format!("document.head.innerHTML=''; document.body.style='margin:0'; document.body.innerHTML={}; scrollTo(0,0); document.addEventListener('submit', e=>e.preventDefault())", json!(html))).await;
    rig
}

async fn read(rig: &Rig, find: Option<&str>) -> Value {
    rig.browser
        .read(
            SESSION,
            ReadOptions {
                find: find.map(str::to_owned),
                ..ReadOptions::default()
            },
            false,
            Actor::Agent,
        )
        .await
        .unwrap()
        .0
}

fn refs(answer: &Value) -> &[Value] {
    answer["untrustedPageContent"]["refs"].as_array().unwrap()
}

#[tokio::test]
async fn each_label_kind_is_searchable_alongside_placeholder_and_value() {
    for (html, query) in [
        (
            "<label for=x>Explicit label</label><input id=x>",
            "Explicit label",
        ),
        (
            "<span id=t>Referenced label</span><input aria-labelledby=t>",
            "Referenced label",
        ),
        ("<label>Wrapping label<input></label>", "Wrapping label"),
        ("<input aria-label='Accessible label'>", "Accessible label"),
        (
            "<label>Account<input placeholder='Email address'></label>",
            "Email address",
        ),
        (
            "<input aria-label='Account' value='Readable value'>",
            "Readable value",
        ),
        (
            "<div><div><label>Nearby label</label></div><div><input></div></div>",
            "Nearby label",
        ),
        ("<div><span>Nearby text</span><input></div>", "Nearby text"),
        (
            "<fieldset><legend>Hobbies</legend><label><input type=checkbox>Reading</label></fieldset>",
            "Hobbies",
        ),
        (
            "<div><label>Subject</label><div><div><input role=combobox></div></div></div>",
            "Subject",
        ),
        (
            "<div><div id=p>Select City</div><div><input role=combobox aria-describedby=p></div></div>",
            "Select City",
        ),
        (
            "<div><div class=custom__single-value>NCR</div><div><input role=combobox aria-label=State></div></div>",
            "NCR",
        ),
    ] {
        let rig = page(html).await;
        let answer = read(&rig, Some(query)).await;
        assert!(!refs(&answer).is_empty(), "{query}: {answer}");
    }
}

#[tokio::test]
async fn saved_practice_form_exposes_labels_and_controls() {
    let rig = page(include_str!("support/practice_form.html")).await;
    for query in [
        "Date of Birth",
        "hobbies",
        "subject",
        "Select State",
        "Select City",
    ] {
        let answer = read(&rig, Some(query)).await;
        assert!(!refs(&answer).is_empty(), "{query}: {answer}");
    }
    let answer = read(&rig, None).await;
    assert!(
        refs(&answer).iter().any(|r| r["role"] == "textbox"),
        "full read missed inputs: {answer}"
    );
    assert!(
        refs(&answer).iter().any(|r| r["name"]
            .as_str()
            .is_some_and(|s| s.contains("Date of Birth"))),
        "full read missed date field: {answer}"
    );
}

#[tokio::test]
async fn form_and_main_content_precede_long_navigation_lists() {
    let mut links = String::new();
    for i in 0..1500 {
        write!(links, "<a href='/nav/{i}'>Navigation {i}</a><br>").unwrap();
    }
    let rig = page(&format!("<nav>{links}</nav><main><a href='/article'>Main article</a><form><label>Date of Birth<input></label><label>Subjects<input role=combobox></label></form></main>")).await;
    let answer = read(&rig, None).await;
    for name in ["Date of Birth", "Subjects", "Main article"] {
        assert!(
            refs(&answer).iter().any(|r| r["name"] == name),
            "{name} starved: {answer}"
        );
    }
    assert!(
        answer["trimmed"]["how"].is_string()
            || answer["note2"].as_str().is_some_and(|s| s.contains("find")),
        "missing continuation: {answer}"
    );
}

#[tokio::test]
async fn hidden_checkbox_label_has_an_actionable_ref() {
    let rig = page("<style>input{opacity:0;position:absolute}</style><label for=x>Visible choice</label><input id=x type=checkbox>").await;
    let answer = read(&rig, Some("Visible choice")).await;
    assert!(
        refs(&answer).iter().any(|r| r["visible"] != false),
        "visible label was missed: {answer}"
    );
    let label = refs(&answer)
        .iter()
        .find(|r| r["visible"] != false)
        .unwrap();
    let action = vw_browser::parse(&json!({"action":"click", "ref":label["ref"]})).unwrap();
    let result = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await;
    assert!(result.is_ok(), "label must be safely clickable: {result:?}");
    assert_eq!(
        rig.engine
            .eval("document.querySelector('input').checked")
            .await,
        true
    );
}

#[tokio::test]
async fn selected_custom_values_are_found_without_exposing_secret_values() {
    let rig = page(include_str!("support/practice_form.html")).await;
    rig.engine.eval(r"for (const [id, text] of [['state','NCR'],['city','Delhi']]) {
        const input = document.querySelector('#'+id+' input'); input.disabled=false;
        const value = document.createElement('div'); value.className='custom__single-value'; value.textContent=text;
        input.parentElement.before(value);
    }").await;
    for value in ["NCR", "Delhi"] {
        let answer = read(&rig, Some(value)).await;
        assert!(
            refs(&answer).iter().any(|r| r["role"] == "combobox"),
            "{value}: {answer}"
        );
    }
    let secret = page("<label>Password<input type=password value='private-phrase'></label><label>Card number<input value='4111111111111111'></label><label>One-time code<input value='934821'></label><div><label>Card number</label><div><input value='5555555555554444'></div></div><div><span>One-time code</span><input value='318927'></div>").await;
    for value in [
        "private-phrase",
        "4111111111111111",
        "934821",
        "5555555555554444",
        "318927",
    ] {
        let answer = read(&secret, Some(value)).await;
        assert!(
            refs(&answer).is_empty(),
            "find must not leak a secret through matching: {answer}"
        );
    }
}
