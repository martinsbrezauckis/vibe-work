//! Focused browser reads retain useful facts under the whole-result budget.
//! Exercises production scripts and final fitting through the isolated Chromium fixture.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions, observe};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

async fn read(rig: &Rig, options: ReadOptions) -> Value {
    let mut out = rig
        .browser
        .read(SESSION, options, false, Actor::Agent)
        .await
        .unwrap()
        .0;
    out["tab"] = json!({"id":"web-page-fixture","site":"93.184.215.14"});
    out["untrustedPageContent"]["tab"] = json!({"title":out["untrustedPageContent"]["title"],"url":out["untrustedPageContent"]["url"]});
    observe::fit(&mut out, observe::READ_BUDGET);
    out
}

#[tokio::test]
async fn heading_facts_distinguish_a_title_from_an_absent_main_heading() {
    let rig = Rig::new().await;
    rig.engine.eval("document.title='Index of available pages'; document.body.innerHTML='<a href=/form>Form</a>'").await;
    let empty = read(&rig, ReadOptions::default()).await;
    assert_eq!(
        empty["untrustedPageContent"]["mainHeadings"],
        json!([]),
        "{empty}"
    );
    rig.engine.eval("document.body.innerHTML='<h1>Visible heading</h1><h1 hidden>Hidden heading</h1><div role=heading aria-level=1>Accessible heading</div><h2>Subheading</h2>'").await;
    let out = read(&rig, ReadOptions::default()).await;
    assert_eq!(
        out["untrustedPageContent"]["mainHeadings"],
        json!(["Visible heading", "Accessible heading"]),
        "{out}"
    );
}

#[tokio::test]
async fn text_only_read_does_not_spend_its_budget_on_ordinary_refs() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<article>'+ 'Readable paragraph. '.repeat(100)+'</article>'+Array.from({length:40},(_,i)=>'<button>Control '+i+'</button>').join('')").await;
    let out = read(
        &rig,
        ReadOptions {
            max_refs: 0,
            text_page: Some(0),
            ..ReadOptions::default()
        },
    )
    .await;
    assert_eq!(out["untrustedPageContent"]["refs"], json!([]), "{out}");
    assert!(
        out["untrustedPageContent"]["text"].as_str().unwrap().len() >= 1000,
        "{out}"
    );
}

#[tokio::test]
async fn a_missing_control_offers_an_inline_screenshot_without_creating_files() {
    let rig = Rig::new().await;
    let out = read(
        &rig,
        ReadOptions {
            find: Some("missing control".into()),
            ..ReadOptions::default()
        },
    )
    .await;
    let next = out["nextStep"].as_str().unwrap();
    assert!(next.contains("screenshot:true"), "{next}");
    assert!(!next.contains("browser_capture"), "{next}");
}

#[tokio::test]
async fn a_large_choice_inventory_cannot_hide_the_focused_control_or_all_text() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<p>Forecast city and tomorrow</p><button>Forecast city</button><iframe src=\"https://consent.example.test/privacy\" style=\"position:fixed;inset:0;width:100%;height:100%;border:0\"></iframe>'").await;
    rig.engine
        .wait_for_frame("https://consent.example.test/privacy", "#reject")
        .await;
    rig.engine.eval_in_frame_pages("if(window!==top) document.body.innerHTML='<h1>Privacy choices</h1><button>Accept</button><button>Settings</button>'+Array.from({length:30},(_,i)=>'<button aria-expanded=false>Detailed purpose '+i+' '+ 'descriptive text '.repeat(5)+'</button>').join('')").await;
    let out = read(
        &rig,
        ReadOptions {
            find: Some("Forecast city".into()),
            ..ReadOptions::default()
        },
    )
    .await;
    assert!(
        out["untrustedPageContent"]["refs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "Forecast city"),
        "{out}"
    );
    assert!(
        out["untrustedPageContent"]["text"]
            .to_string()
            .contains("Forecast city"),
        "{out}"
    );
    assert_eq!(
        out["untrustedPageContent"]["coveringFrames"][0]["complete"],
        false
    );
}

#[tokio::test]
async fn text_cursors_recover_every_character_across_byte_trimming_and_frame_windows() {
    let rig = Rig::new().await;
    rig.engine.viewport(1280, 900).await;
    let main = format!("{}🦊{}", "a ".repeat(1249) + "x", "b ".repeat(1600));
    let child = "Child paragraph. ".repeat(200);
    rig.engine.eval(&format!("document.body.innerHTML='<pre id=main></pre><iframe src=\"https://consent.example.test/privacy\" style=\"width:300px;height:200px;position:fixed;top:0;left:0\"></iframe>'; document.querySelector('#main').textContent={}", json!(main))).await;
    rig.engine
        .wait_for_frame("https://consent.example.test/privacy", "#reject")
        .await;
    rig.engine.eval_in_frame_pages(&format!("if(window!==top) {{ document.body.innerHTML='<pre></pre>'; document.querySelector('pre').textContent={}; }}", json!(child))).await;
    let mut cursor = vw_browser::TextCursor::default();
    let mut windows = std::collections::BTreeMap::<usize, String>::new();
    for iteration in 0..40 {
        let out = read(
            &rig,
            ReadOptions {
                max_refs: 0,
                text_cursor: Some(cursor.clone()),
                ..ReadOptions::default()
            },
        )
        .await;
        let content = &out["untrustedPageContent"];
        let text = content["text"].as_str().unwrap();
        assert!(!text.is_empty(), "nonprogress at {iteration}: {out}");
        windows.entry(cursor.page).or_default().push_str(text);
        let Some(next) = content.get("nextTextCursor") else {
            break;
        };
        let next: vw_browser::TextCursor = serde_json::from_value(next.clone()).unwrap();
        assert!(
            (next.page, next.offset) > (cursor.page, cursor.offset),
            "{out}"
        );
        cursor = next;
        assert!(iteration < 39, "continuation loop");
    }
    // Windows contain each visible document's slice in order, separated by a newline.
    let slice = |text: &str, page: usize| {
        let mut at = 0;
        text.chars()
            .filter(|c| {
                let start = at;
                at += c.len_utf16();
                start >= page * 2500 && start < (page + 1) * 2500
            })
            .collect::<String>()
    };
    for page in 0..3 {
        let expected = [slice(&main, page), slice(&child, page)]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(windows.get(&page).unwrap(), &expected, "text page {page}");
    }
}

#[tokio::test]
async fn choice_pages_recover_every_exact_name_without_repeating_background_text() {
    let rig = Rig::new().await;
    rig.engine.viewport(1280, 900).await;
    rig.engine.eval("document.body.innerHTML='<main>'+ 'Background article. '.repeat(300)+'</main><iframe src=\"https://consent.example.test/privacy\" style=\"position:fixed;inset:0;width:100%;height:100%;border:0\"></iframe>'").await;
    rig.engine
        .wait_for_frame("https://consent.example.test/privacy", "#reject")
        .await;
    let expected: Vec<_> = (0..30)
        .map(|i| format!("Purpose {i}: {}", "rendered terms ".repeat(4).trim()))
        .collect();
    rig.engine.eval_in_frame_pages(&format!("if(window!==top) document.body.innerHTML={}.map(name=>'<button>'+name+'</button>').join('')", json!(expected))).await;
    let mut out = read(&rig, ReadOptions::default()).await;
    let mut names = Vec::new();
    let mut pages = 0;
    loop {
        let frame = &out["untrustedPageContent"]["coveringFrames"][0];
        names.extend(
            frame["choices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["name"].as_str().unwrap().to_owned()),
        );
        let Some(next) = frame.get("nextChoiceCursor") else {
            break;
        };
        let cursor: vw_browser::ChoiceCursor = serde_json::from_value(next.clone()).unwrap();
        assert_eq!(cursor.offset, names.len(), "{out}");
        out = read(
            &rig,
            ReadOptions {
                choice_cursor: Some(cursor),
                ..ReadOptions::default()
            },
        )
        .await;
        assert!(out["untrustedPageContent"].get("text").is_none(), "{out}");
        assert_eq!(out["untrustedPageContent"]["refs"], json!([]));
        pages += 1;
        assert!(pages < 20, "choice paging loop");
    }
    assert!(pages > 0, "fixture must exercise continuation: {out}");
    assert_eq!(names, expected);
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[tokio::test]
async fn main_document_consent_choices_continue_independently_of_background_links_and_find() {
    let rig = Rig::new().await;
    rig.engine.viewport(1280, 900).await;
    let expected: Vec<_> = (0..30)
        .map(|i| {
            format!(
                "Privacy purpose {i}: {}",
                "rendered terms ".repeat(4).trim()
            )
        })
        .collect();
    rig.engine.eval(&format!("document.body.innerHTML=Array.from({{length:240}},(_,i)=>'<a href=/news/'+i+'>Background '+i+'</a>').join('')+'<div role=dialog aria-modal=true style=\"position:fixed;inset:0;background:white;overflow:auto\"><h2>Privacy choices</h2>'+{}.map(name=>'<button>'+name+'</button>').join('')+'<button hidden>Hidden purpose</button></div>'", json!(expected))).await;
    let mut out = read(
        &rig,
        ReadOptions {
            find: Some("no ordinary match".into()),
            ..ReadOptions::default()
        },
    )
    .await;
    let mut names = Vec::new();
    for iteration in 0..20 {
        let frame = &out["untrustedPageContent"]["coveringFrames"][0];
        let choices = frame["choices"]
            .as_array()
            .expect("main-document wall needs a choice inventory");
        names.extend(
            choices
                .iter()
                .map(|c| c["name"].as_str().unwrap().to_owned()),
        );
        let Some(next) = frame.get("nextChoiceCursor") else {
            break;
        };
        let cursor: vw_browser::ChoiceCursor = serde_json::from_value(next.clone()).unwrap();
        assert_eq!(cursor.offset, names.len());
        out = read(
            &rig,
            ReadOptions {
                choice_cursor: Some(cursor),
                ..ReadOptions::default()
            },
        )
        .await;
        assert!(iteration < 19, "choice paging loop");
    }
    assert_eq!(names, expected);
    let reference = out["untrustedPageContent"]["coveringFrames"][0]["choices"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["ref"]
        .as_str()
        .unwrap();
    assert!(rig.browser.observed_target(SESSION, reference).is_some());
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    rig.engine.eval("document.querySelector('[role=dialog] button:last-of-type').previousElementSibling.onclick=()=>document.body.dataset.chosen='yes'").await;
    let action = vw_browser::Action::Click {
        reference: reference.into(),
    };
    let receipt = rig
        .browser
        .act(SESSION, action.clone(), Actor::Agent, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(receipt["ok"], true, "{receipt}");
    assert_eq!(rig.engine.eval("document.body.dataset.chosen").await, "yes");
    rig.engine.eval("const b=document.querySelector('[role=dialog] button:last-of-type').previousElementSibling;b.replaceWith(b.cloneNode(true))").await;
    assert!(
        rig.browser
            .act(SESSION, action, Actor::Agent, &NoNavigation)
            .await
            .is_err()
    );
}
