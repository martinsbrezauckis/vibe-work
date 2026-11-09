//! Real-page result budgets, control priority and code-mode batches through the headless browser driver.
//! Uses offline news/article pages and the saved practice form; includes the tool's tab envelope.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use std::fmt::Write as _;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions, observe};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

fn news() -> String {
    let mut html = String::from("<title>Independent news</title><nav>");
    for i in 0..224 {
        write!(html, "<a href='/world/story-{i}/report-analysis'>World report {i}: independent reporting and analysis from correspondents around the world</a> ").unwrap();
    }
    html.push_str("</nav><iframe title='Privacy choices' src='https://consent.example.test/privacy' style='position:fixed;left:20px;top:30px;width:600px;height:350px'></iframe>");
    html
}

fn article() -> String {
    let mut html =
        String::from("<title>A long report</title><article><button>Save article</button>");
    for i in 0..120 {
        write!(
            html,
            "<p>Paragraph {i}: {}.</p>",
            "Journalists describe the changing landscape — café 🌍. ".repeat(12)
        )
        .unwrap();
    }
    html.push_str("</article>");
    html
}

async fn page(html: &str) -> Rig {
    let rig = Rig::new().await;
    rig.engine.viewport(1280, 800).await;
    rig.engine.eval(&format!("document.head.innerHTML=''; document.body.style='margin:0'; document.body.innerHTML={}; scrollTo(0,0); document.addEventListener('submit', e=>e.preventDefault())", json!(html))).await;
    rig
}

fn envelope(answer: &mut Value, budget: usize) {
    let content = &answer["untrustedPageContent"];
    let tab = json!({"title":content["title"],"url":content["url"]});
    answer["tab"] = json!({"id":"3f2b7c1e-9a4d-4e8b-b6f1-0c5d2a7e9b13","site":"93.184.215.14"});
    answer["untrustedPageContent"]["tab"] = tab;
    observe::fit(answer, budget);
    vw_browser::filter_agent_output(answer);
}

async fn read(rig: &Rig, options: ReadOptions) -> Value {
    let budget = options.max_bytes;
    let mut answer = rig
        .browser
        .read(SESSION, options, false, Actor::Agent)
        .await
        .unwrap()
        .0;
    envelope(&mut answer, budget);
    answer
}

fn sizes(label: &str, value: &Value) -> (usize, usize) {
    let compact = value.to_string().len();
    let pretty = serde_json::to_string_pretty(value).unwrap().len();
    println!("SIZE {label}: compact={compact}, pretty={pretty}");
    (compact, pretty)
}

fn refs(value: &Value) -> &[Value] {
    value["untrustedPageContent"]["refs"].as_array().unwrap()
}

#[test]
fn byte_trimming_keeps_main_refs_and_explains_the_reservation() {
    for role in ["textbox", "link"] {
        let mut answer = json!({"refsTotal":100,"refsShown":32,"untrustedPageContent":{"refs":[]}});
        let items = answer["untrustedPageContent"]["refs"]
            .as_array_mut()
            .unwrap();
        for i in 0..32 {
            items.push(
                json!({"ref":format!("e{i:08x}"), "role":if i < 16 { "button" } else { role },
                "name":format!("Control {i} {}", "text ".repeat(12)), "frame":i < 16}),
            );
        }
        observe::fit(&mut answer, 1500);
        let items = refs(&answer);
        assert_ne!(items, &[] as &[Value]);
        assert!(
            items.iter().filter(|r| r["frame"] != true).count() >= items.len().div_ceil(2),
            "{answer}"
        );
        assert_eq!(items[0]["frame"], true);
        assert!(
            answer["trimmed"]["how"].as_str().unwrap().contains("half"),
            "{answer}"
        );
        assert!(answer.to_string().len() <= 1500);
    }
}

#[tokio::test]
async fn focused_reads_keep_main_matches_beside_frame_matches() {
    let rig = page("<form><input aria-label='Choice main'></form><iframe src='https://consent.example.test/privacy' style='position:fixed;inset:0;width:100%;height:100%;border:0'></iframe>").await;
    rig.engine
        .wait_for_frame("https://consent.example.test/privacy", "#reject")
        .await;
    rig.engine.eval_in_frame_pages("if(window!==top) document.body.innerHTML='<div role=dialog>'+Array.from({length:100},(_,i)=>'<button>Choice '+i+'</button>').join('')+'</div>'").await;
    let answer = read(
        &rig,
        ReadOptions {
            find: Some("Choice".into()),
            ..ReadOptions::default()
        },
    )
    .await;
    assert!(
        refs(&answer).iter().any(|r| r["name"] == "Choice main"),
        "{answer}"
    );
    assert_eq!(refs(&answer)[0]["frame"], true);
}

async fn fixture_budget(label: &str, html: &str) {
    let rig = page(html).await;
    if label == "news" {
        rig.engine
            .wait_for_frame("https://consent.example.test/privacy", "#reject")
            .await;
    }
    let mut results = Vec::new();
    for (name, max_bytes, max_refs) in [
        ("default", 3000, 32),
        ("walk", 30_000, 100),
        ("maximum", 64_000, 200),
    ] {
        let answer = read(
            &rig,
            ReadOptions {
                max_bytes,
                max_refs,
                ..ReadOptions::default()
            },
        )
        .await;
        results.push((
            sizes(&format!("{label}/{name}"), &answer),
            answer,
            name != "default",
        ));
    }
    for ((compact, pretty), answer, clamped) in results {
        assert!(
            compact <= 9000 && pretty < 15_000,
            "{label}: {compact}/{pretty}"
        );
        assert!(
            compact <= 4000 && pretty <= 4500,
            "individual inline ceiling: {compact}/{pretty}"
        );
        assert!(answer["snapshot"].is_string());
        assert!(answer["promptGuard"].is_object());
        assert!(answer["guardLayers"].is_object() || answer["guardLayers"].is_array());
        assert_eq!(answer["note"], observe::UNTRUSTED_NOTE);
        assert_eq!(answer["size"]["bytes"], compact);
        assert_eq!(
            answer.to_string().matches("clamped").count(),
            usize::from(clamped)
        );
        if label == "news" {
            assert_eq!(answer["refsTotal"], 227);
            for name in ["Accept all", "Reject optional cookies", "Manage choices"] {
                let control = refs(&answer)
                    .iter()
                    .find(|r| r["name"] == name)
                    .expect("consent controls must survive links and text");
                assert_eq!(control["frameOrigin"], "https://consent.example.test");
            }
        }
    }
}

#[tokio::test]
async fn news_results_fit_with_consent_controls() {
    fixture_budget("news", &news()).await;
}

#[tokio::test]
async fn long_article_results_fit() {
    fixture_budget("article", &article()).await;
}

#[tokio::test]
async fn practice_form_results_fit() {
    fixture_budget("form", include_str!("support/practice_form.html")).await;
}

#[tokio::test]
async fn read_act_read_code_mode_stays_inline() {
    let rig = page(include_str!("support/practice_form.html")).await;
    for max_bytes in [3000, 64_000] {
        let options = ReadOptions {
            max_bytes,
            max_refs: 200,
            ..ReadOptions::default()
        };
        let before = read(&rig, options.clone()).await;
        // Use a focused read only to locate the action; the combined return contains full reads.
        let focused = read(
            &rig,
            ReadOptions {
                find: Some("First Name".into()),
                ..ReadOptions::default()
            },
        )
        .await;
        let field = refs(&focused)
            .iter()
            .find(|r| r["role"] == "textbox")
            .unwrap();
        let action =
            vw_browser::parse(&json!({"action":"type","ref":field["ref"],"text":"Sample"}))
                .unwrap();
        let mut act = rig
            .browser
            .act(SESSION, action, Actor::Agent, &NoNavigation)
            .await
            .unwrap();
        envelope(&mut act, 4000);
        let after = read(&rig, options).await;
        let (compact, pretty) = sizes("action", &act);
        assert!(compact <= 4000 && pretty < 15_000);
        let combined = json!({"read":before,"act":act,"after":after});
        let (_, pretty) = sizes(&format!("combined/{max_bytes}"), &combined);
        assert!(pretty < 16_000, "combined code-mode return: {pretty}");
    }
}

#[tokio::test]
async fn shortened_article_explains_supported_continuation() {
    let rig = page(&article()).await;
    let answer = read(
        &rig,
        ReadOptions {
            text_page: Some(0),
            max_bytes: 1500,
            ..ReadOptions::default()
        },
    )
    .await;
    sizes("article/textPage/minimum", &answer);
    let how = answer["trimmed"]["how"].as_str().unwrap();
    assert!(
        !how.contains("64000"),
        "must not suggest bypassing the inline cap: {how}"
    );
    assert!(how.contains("textCursor") && how.contains("find"));
    assert!(answer["untrustedPageContent"]["nextTextCursor"].is_object());
    assert!(
        how.contains('0'),
        "shortened current window must remain recoverable: {how}"
    );
}

#[tokio::test]
async fn full_news_ref_remains_actionable_after_a_focused_read() {
    let rig = page(&news()).await;
    rig.engine
        .wait_for_frame("https://consent.example.test/privacy", "#reject")
        .await;
    let before = read(&rig, ReadOptions::default()).await;
    let reject = refs(&before)
        .iter()
        .find(|r| r["name"] == "Reject optional cookies")
        .unwrap();
    let late = read(
        &rig,
        ReadOptions {
            find: Some("World report 223".into()),
            ..ReadOptions::default()
        },
    )
    .await;
    assert_eq!(
        refs(&late).len(),
        1,
        "find recovers a link omitted from the full read"
    );
    let action = vw_browser::parse(&json!({"action":"click","ref":reject["ref"]})).unwrap();
    let mut receipt = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await
        .unwrap();
    envelope(&mut receipt, 4000);
    assert_eq!(receipt["ok"], true);
    assert!(sizes("consent action", &receipt).0 <= 4000);
}

#[tokio::test]
async fn focused_text_can_continue_beyond_the_first_search_window() {
    let rig = page(&article()).await;
    let offset = rig
        .engine
        .eval("document.body.innerText.indexOf('Paragraph 100:')")
        .await
        .as_u64()
        .unwrap();
    assert!(offset > 20_000);
    let page_number = usize::try_from(offset / 2500).unwrap();
    let answer = read(
        &rig,
        ReadOptions {
            text_page: Some(page_number),
            find: Some("Paragraph 100".into()),
            ..ReadOptions::default()
        },
    )
    .await;
    assert!(
        answer["untrustedPageContent"]["text"]
            .to_string()
            .contains("Paragraph 100")
    );
    assert_eq!(
        answer["untrustedPageContent"]["textPageNumber"],
        page_number
    );
}
