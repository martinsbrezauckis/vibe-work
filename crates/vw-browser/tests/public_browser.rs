//! Opt-in deterministic measurements on public pages through the production browser driver.
//! Writes raw receipts for the private bench harness; no accounts, submission or consent acceptance.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use std::time::Instant;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

struct Navigation<'a>(&'a Rig);
impl vw_browser::Navigate for Navigation<'_> {
    fn open<'a>(&'a self, _: &'a str, url: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async move {
            self.0
                .service
                .open(vw_preview::OpenRequest {
                    session_id: SESSION.into(),
                    url: url.parse().map_err(|e| format!("{e}"))?,
                    allowed_ports: vec![],
                    downloads_dir: None,
                    wait: true,
                    lan: false,
                })
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    }
}

struct Walk {
    rig: Rig,
    calls: Vec<Value>,
    start: Instant,
}

impl Walk {
    async fn new() -> Self {
        let rig = Rig::new().await;
        rig.engine.viewport(1280, 900).await;
        rig.engine.public_pages().await;
        Self {
            rig,
            calls: vec![],
            start: Instant::now(),
        }
    }

    fn record(&mut self, tool: &str, args: Value, value: Value, start: Instant) -> Value {
        let mut call = json!({"tool":tool,"result":value,
            "seconds":start.elapsed().as_secs_f64(),"bytes":value.to_string().len(),
            "prettyBytes":serde_json::to_string_pretty(&value).unwrap().len()});
        call["args"] = args;
        self.calls.push(call);
        value
    }

    async fn read(&mut self, find: Option<&str>) -> Value {
        self.read_options(
            ReadOptions {
                find: find.map(str::to_owned),
                ..ReadOptions::default()
            },
            json!({"find":find}),
        )
        .await
    }

    async fn read_options(&mut self, options: ReadOptions, args: Value) -> Value {
        let start = Instant::now();
        let result = self
            .rig
            .browser
            .read(SESSION, options, false, Actor::Agent)
            .await
            .map_or_else(|e| json!({"error":e}), |r| r.0);
        self.record("browser_read", args, result, start)
    }

    async fn act(&mut self, args: Value) -> Value {
        let start = Instant::now();
        let result = self
            .rig
            .browser
            .act(
                SESSION,
                vw_browser::parse(&args).unwrap(),
                Actor::Agent,
                &Navigation(&self.rig),
            )
            .await
            .unwrap_or_else(|e| json!({"error":e.text,"outcome":format!("{:?}",e.outcome)}));
        self.record("browser_act", args, result, start)
    }

    fn finish(self, id: &str, verified: bool, detail: Value) -> Value {
        let mut result = json!({"id":id,"verified":verified,"seconds":self.start.elapsed().as_secs_f64(),
            "browserCalls":self.calls.len(),"steps":self.calls.len(),
            "reads":self.calls.iter().filter(|c| c["tool"] == "browser_read").count(),
            "finds":self.calls.iter().filter(|c| c["tool"] == "browser_read" && c["args"]["find"].is_string()).count(),
            "bytes":self.calls.iter().map(|c| c["bytes"].as_u64().unwrap()).sum::<u64>(),
            "receiptsOverInlineLimit":self.calls.iter().filter(|c| c["prettyBytes"].as_u64().unwrap() > 15000).count(),
            "spills":null,"cards":null,"modelSteps":null,"calls":self.calls});
        result["detail"] = detail;
        result
    }
}

#[tokio::test]
#[ignore = "real public consent inventory; run explicitly with VW_BROWSER_BENCH_OUTPUT"]
async fn public_choice_continuation() {
    let output = std::env::var("VW_BROWSER_BENCH_OUTPUT").expect("receipt output path");
    let mut walk = Walk::new().await;
    walk.act(json!({"action":"navigate","url":"https://www.meteounradars.lv/"}))
        .await;
    walk.act(json!({"action":"wait","text":"PIEŅEMT","timeoutMs":15000}))
        .await;
    let mut answer = walk.read(None).await;
    let (model, _, _) = walk
        .rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let oracle: Vec<_> = model
        .covering_frames
        .first()
        .map(|frame| {
            frame
                .choices
                .iter()
                .map(|choice| choice.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let mut names = Vec::new();
    for _ in 0..20 {
        let frame = &answer["untrustedPageContent"]["coveringFrames"][0];
        if let Some(choices) = frame["choices"].as_array() {
            names.extend(
                choices
                    .iter()
                    .filter_map(|choice| choice["name"].as_str().map(str::to_owned)),
            );
        }
        let Some(next) = frame.get("nextChoiceCursor").cloned() else {
            break;
        };
        let cursor = serde_json::from_value(next.clone()).unwrap();
        answer = walk
            .read_options(
                ReadOptions {
                    choice_cursor: Some(cursor),
                    ..ReadOptions::default()
                },
                json!({"choiceCursor":next}),
            )
            .await;
    }
    let verified = !oracle.is_empty() && names == oracle;
    let result = walk.finish("CONSENT-PAGES", verified, json!({"scope":"Read-only choice pagination, not a forecast or model run. Oracle is a separate full host observation, excluded from tool counts.","names":names,"oracleNames":oracle}));
    std::fs::write(output, serde_json::to_string_pretty(&result).unwrap()).unwrap();
    assert!(
        verified,
        "public consent inventory changed or was unavailable"
    );
}

fn reference(answer: &Value, name: &str) -> Option<Value> {
    ["refs", "newRefs"].iter().find_map(|key| {
        answer["untrustedPageContent"][key]
            .as_array()
            .and_then(|refs| {
                refs.iter()
                    .find(|r| r["name"].as_str().is_some_and(|n| n.contains(name)))
            })
            .map(|r| r["ref"].clone())
    })
}

async fn selenium_date() -> Value {
    let mut walk = Walk::new().await;
    walk.act(
        json!({"action":"navigate","url":"https://www.selenium.dev/selenium/web/web-form.html"}),
    )
    .await;
    let read = walk.read(Some("Date picker")).await;
    if let Some(reference) = reference(&read, "Date picker") {
        walk.act(json!({"action":"type","ref":reference,"text":"03/14/2026"}))
            .await;
        walk.act(json!({"action":"press","ref":reference,"key":"Tab"}))
            .await;
    }
    let fresh = walk.read(Some("Date picker")).await;
    let actual = walk
        .rig
        .engine
        .eval("document.querySelector('[name=my-date]')?.value")
        .await;
    walk.finish(
        "DATE-TEXT",
        actual == "03/14/2026",
        json!({"oracle":actual,"freshRead":fresh}),
    )
}

async fn consent_manager() -> Value {
    let mut walk = Walk::new().await;
    walk.act(json!({"action":"navigate","url":"https://www.theguardian.com/europe"}))
        .await;
    walk.act(json!({"action":"wait","text":"Accept all","timeoutMs":15000}))
        .await;
    let first = walk.read(Some("privacy")).await;
    let choice = first["untrustedPageContent"]["coveringFrames"]
        .as_array()
        .and_then(|frames| {
            frames
                .iter()
                .flat_map(|f| f["choices"].as_array().into_iter().flatten())
                .find(|r| r["name"].as_str().is_some_and(|n| n.ends_with(" partners")))
                .map(|r| r["ref"].clone())
        });
    if let Some(reference) = choice {
        walk.act(json!({"action":"click","ref":reference})).await;
    }
    let read = walk.read(None).await;
    let states = read["untrustedPageContent"]["coveringFrames"]
        .as_array()
        .map_or(0, |frames| {
            frames
                .iter()
                .flat_map(|f| f["choices"].as_array().into_iter().flatten())
                .filter(|r| r.get("checked").is_some() || r.get("pressed").is_some())
                .count()
        });
    let frame = &read["untrustedPageContent"]["coveringFrames"][0];
    let complete = frame["complete"] == true
        && frame["choices"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|r| r["name"] == "Save and close" && r["disabled"] == true)
        });
    walk.finish(
        "T8-MANAGER",
        complete,
        json!({"toggleStates":states,"read":read,
        "scope":"Opened the partner list only; no purpose toggled or saved."}),
    )
}

async fn selenium() -> Value {
    let mut walk = Walk::new().await;
    walk.act(
        json!({"action":"navigate","url":"https://www.selenium.dev/selenium/web/web-form.html"}),
    )
    .await;
    let read = walk.read(None).await;
    for (name, mut args) in [
        ("Text input", json!({"action":"type","text":"Test User"})),
        (
            "Textarea",
            json!({"action":"type","text":"Browser extension test"}),
        ),
        (
            "Dropdown (select)",
            json!({"action":"select","value":"Two"}),
        ),
        ("Default checkbox", json!({"action":"check","checked":true})),
    ] {
        let Some(reference) = reference(&read, name) else {
            continue;
        };
        args["ref"] = reference;
        walk.act(args).await;
    }
    let verified = walk.read(None).await;
    let oracle = walk.rig.engine.eval("({title:document.title,url:location.href,heading:document.querySelector('h1')?.innerText,text:document.querySelector('[name=my-text]')?.value,textarea:document.querySelector('textarea')?.value,selected:document.querySelector('select')?.selectedOptions[0]?.text,checked:document.querySelector('#my-check-2')?.checked})").await;
    let pass = oracle["text"] == "Test User"
        && oracle["textarea"] == "Browser extension test"
        && oracle["selected"] == "Two"
        && oracle["checked"] == true;
    walk.finish(
        "QA1-QA2",
        pass,
        json!({"oracle":oracle,"freshRead":verified}),
    )
}

async fn datepicker() -> Value {
    let mut walk = Walk::new().await;
    walk.act(json!({"action":"navigate","url":"https://demoqa.com/date-picker"}))
        .await;
    let read = walk.read(Some("Select Date")).await;
    if let Some(reference) = reference(&read, "Select Date") {
        walk.act(json!({"action":"click","ref":reference})).await;
    }
    let month = walk.read(Some("November")).await;
    let march = walk.read(Some("March")).await;
    if let Some(reference) = reference(&march, "") {
        walk.act(json!({"action":"select","ref":reference,"value":"March"}))
            .await;
    }
    let oracle = walk.rig.engine.eval("Array.from(document.querySelectorAll('select')).map(s=>({value:s.value,selected:s.selectedOptions[0]?.text}))").await;
    let pass = oracle
        .as_array()
        .is_some_and(|items| items.iter().any(|s| s["selected"] == "March"));
    walk.finish("DATE", pass, json!({"initialMonth":month,"oracle":oracle}))
}

async fn inspect(id: &str, url: &str, query: Option<&str>) -> Value {
    let mut walk = Walk::new().await;
    let opened = walk.act(json!({"action":"navigate","url":url})).await;
    // Dynamic public widgets need time to render; this is one explicit browser tool step.
    walk.act(json!({"action":"wait","load":true,"timeoutMs":15000}))
        .await;
    let read = walk.read(query).await;
    let pass = opened["ok"] == true && read["untrustedPageContent"].is_object();
    walk.finish(id, pass, json!({"scope":"read-only page availability; full colleague/model task not measured","read":read}))
}

#[tokio::test]
#[ignore = "real public pages; run explicitly with VW_BROWSER_BENCH_OUTPUT"]
async fn public_page_measurements() {
    let output = std::env::var("VW_BROWSER_BENCH_OUTPUT").expect("receipt output path");
    let rows = vec![
        selenium().await,
        selenium_date().await,
        datepicker().await,
        inspect("QA3", "https://www.selenium.dev/documentation/", None).await,
        inspect("QA5", "https://www.meteounradars.lv/", Some("Ogre")).await,
        inspect("QA6", "https://www.teacherspayteachers.com/Product/Addition-and-Subtraction-Mental-Math-Quiz-4679741", None).await,
        inspect("T8", "https://www.theguardian.com/europe", Some("privacy")).await,
        consent_manager().await,
    ];
    std::fs::write(output, serde_json::to_string_pretty(&rows).unwrap()).unwrap();
}
