//! The default `browser_read` answer keeps compact control-ref density: frame identity and geometry stay in the host's
//! observation, the agent's text carries only what it acts on (a frame's origin, a visual region's box).
//! Uses the shared real-page engine through the agent's read path; frame and visual refs must stay actionable.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions, model::PageModel, observe};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

/// The app end-to-end test's shop page without its scripts: 7 controls.
const SHOP: &str = r#"<main><h1>Fixture shop</h1>
<form id="f"><label for="name">Name</label> <input id="name" name="name"><br>
<label for="email">Email</label> <input id="email" type="email" name="email"><br>
<label for="pw">Password</label> <input id="pw" type="password"><br>
<button type="submit">Send</button></form><p id="out"></p>
<div style="position:relative;display:inline-block"><button id="buy">Buy now</button>
<div style="position:absolute;inset:0;background:#0004" role="dialog" aria-label="Cookie banner"></div></div>
<p><a href="http://10.255.255.1/">Router admin</a></p><p id="later">Slow content arrived</p>
<button id="probe">Probe</button><p id="probed"></p><p>webtransport undefined</p></main>"#;

/// An ordinary catalogue page with 32 controls: navigation links, a search, product links and buttons.
fn catalogue() -> String {
    use std::fmt::Write as _;
    let mut html = String::from("<nav>");
    for name in [
        "Home", "New in", "Women", "Men", "Kids", "Sale", "Help", "Account",
    ] {
        let path = name.to_lowercase().replace(' ', "-");
        write!(html, "<a href=\"/c/{path}\">{name}</a> ").unwrap();
    }
    html.push_str("</nav><form><input type=search name=q aria-label=\"Search products\"> <button>Search</button></form><ul>");
    for i in 1..=11 {
        write!(
            html,
            "<li><a href=\"/p/{i}\">Ceramic mug no. {i}</a> <button>Add to cart</button></li>"
        )
        .unwrap();
    }
    html.push_str("</ul>");
    html
}

async fn page(html: &str) -> Rig {
    let rig = Rig::new().await;
    rig.engine.viewport(1280, 800).await;
    rig.service
        .layout(Some((
            SESSION.into(),
            vw_preview::Rect {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 800.0,
            },
        )))
        .await
        .unwrap();
    rig.engine
        .eval(&format!(
            "document.body.innerHTML = {}; scrollTo(0,0)",
            json!(html)
        ))
        .await;
    rig
}

/// What the agent tool adds before its own `fit` (vw-core-api `attach_tab`): the tab's id, site, title and URL.
fn attach_tab(answer: &mut Value) {
    let content = &answer["untrustedPageContent"];
    let tab = json!({"title": content["title"], "url": content["url"]});
    answer["tab"] = json!({"id": "3f2b7c1e-9a4d-4e8b-b6f1-0c5d2a7e9b13", "site": "93.184.215.14"});
    answer["untrustedPageContent"]["tab"] = tab;
    observe::fit(answer, observe::READ_BUDGET);
}

/// The default `browser_read` as the agent tool answers it (driver read, tab facts, the tool's own fit).
async fn agent_read(rig: &Rig) -> Value {
    let mut answer = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .unwrap()
        .0;
    attach_tab(&mut answer);
    answer
}

fn refs(answer: &Value) -> &Vec<Value> {
    answer["untrustedPageContent"]["refs"].as_array().unwrap()
}

/// Reference density for the same observation and host facts, omitting the
/// fields used for frames and image regions (`label`, `coordinateKind`, `frameId`, `box`; the page's
/// `viewport`), with the same `fit`.
fn baseline(answer: &Value, model: &PageModel) -> usize {
    let mut old = model.clone();
    old.viewport = None;
    for r in &mut old.refs {
        r.label = None;
        r.coordinate_kind = None;
        r.frame_id = None;
        r.bounds = None;
    }
    let mut host = answer.clone();
    for key in [
        "snapshot",
        "refsShown",
        "refsTotal",
        "note2",
        "scroll",
        "typingTracker",
        "untrustedPageContent",
        "promptGuard",
        "note",
        "trimmed",
        "size",
    ] {
        host.as_object_mut().unwrap().remove(key);
    }
    let verdict = observe::verdict(&old);
    host.as_object_mut().unwrap().remove("tab");
    let mut out = observe::answer(
        &old,
        &verdict,
        "0123456789abcdef",
        host,
        true,
        observe::READ_BUDGET,
    );
    attach_tab(&mut out);
    refs(&out).len()
}

async fn density(html: &str, controls: usize) {
    let rig = page(html).await;
    let answer = agent_read(&rig).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let shown = refs(&answer).len();
    let old = baseline(&answer, &model);
    println!(
        "{controls}-control page: {shown} refs shown, 0.1.4 serializer {old}, {} bytes, trimmed {}",
        observe::size(&answer),
        answer["trimmed"]
    );
    assert_eq!(model.refs.len(), controls, "fixture controls: {model:?}");
    assert!(
        observe::size(&answer) <= observe::READ_BUDGET,
        "over budget: {answer}"
    );
    assert!(
        shown >= old,
        "default read lists {shown} refs, 0.1.4 listed {old}: {answer}"
    );
    assert_eq!(
        answer["refsShown"],
        json!(shown),
        "refsShown counts the refs actually in the answer: {answer}"
    );
    let cut = answer["trimmed"]["refs"].as_u64().unwrap_or(0);
    assert_eq!(
        shown + usize::try_from(cut).unwrap(),
        model.refs.len(),
        "shown + trimmed = observed: {answer}"
    );
    for r in refs(&answer) {
        for key in ["frameId", "box", "label", "frameOrigin"] {
            assert!(r.get(key).is_none(), "main-frame ref carries {key}: {r}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ordinary_shop_page_lists_every_control() {
    density(SHOP, 7).await;
    let rig = page(SHOP).await;
    assert_eq!(refs(&agent_read(&rig).await).len(), 7, "0.1.4 showed all 7");
}

#[tokio::test(flavor = "multi_thread")]
async fn catalogue_page_lists_at_least_as_many_refs_as_before() {
    density(&catalogue(), 32).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn frame_ref_carries_its_origin_and_stays_actionable() {
    let rig = page("<button>Main button</button><iframe style=\"position:fixed;left:12px;top:40px;width:280px;height:180px\" src=\"https://frame.example.test/login\"></iframe>").await;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let answer = agent_read(&rig).await;
    let main = refs(&answer)
        .iter()
        .find(|r| r["name"] == "Main button")
        .unwrap_or_else(|| panic!("main button: {answer}"));
    for key in ["frame", "frameOrigin", "frameId", "box"] {
        assert!(main.get(key).is_none(), "main ref carries {key}: {main}");
    }
    let checkbox = refs(&answer)
        .iter()
        .find(|r| r["name"] == "Fixture checkbox")
        .unwrap_or_else(|| panic!("frame checkbox: {answer}"));
    assert_eq!(checkbox["frame"], true, "{checkbox}");
    assert_eq!(
        checkbox["frameOrigin"], "https://frame.example.test",
        "{checkbox}"
    );
    assert!(checkbox.get("frameId").is_none(), "{checkbox}");
    assert!(checkbox.get("box").is_none(), "{checkbox}");
    let action =
        vw_browser::parse(&json!({"action":"check","ref":checkbox["ref"],"checked":true})).unwrap();
    let receipt = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await;
    assert!(receipt.is_ok(), "frame checkbox: {receipt:?}");
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    assert!(
        model
            .refs
            .iter()
            .any(|r| r.name == "Fixture checkbox" && r.checked == Some(true)),
        "{model:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn visual_ref_carries_a_compact_box_and_stays_actionable() {
    let rig =
        page("<canvas width=280 height=160 style=\"position:fixed;left:0;top:0\"></canvas>").await;
    rig.engine.eval("const c=document.querySelector('canvas').getContext('2d'); c.fillStyle='white'; c.fillRect(0,0,280,160); c.strokeStyle='black'; c.lineWidth=2; c.strokeRect(20,20,20,20); window.hits=0; document.querySelector('canvas').onclick=()=>window.hits++").await;
    let answer = agent_read(&rig).await;
    let square = refs(&answer)
        .iter()
        .find(|r| r["coordinateKind"] == "checkbox")
        .unwrap_or_else(|| panic!("visual square: {answer}"));
    let rect: Vec<i64> = square["box"]
        .as_array()
        .unwrap_or_else(|| panic!("visual ref box: {square}"))
        .iter()
        .map(|n| n.as_i64().unwrap())
        .collect();
    assert_eq!(rect.len(), 4, "box is [x, y, width, height]: {square}");
    assert!((17..=23).contains(&rect[0]), "{square}");
    assert!(square.get("frameId").is_none(), "{square}");
    // The agent reads a region's text from its screenshot: no local recognition state or text refs.
    assert!(
        answer["untrustedPageContent"]
            .get("textRecognition")
            .is_none(),
        "{answer}"
    );
    assert!(
        !refs(&answer).iter().any(|r| r["coordinateKind"] == "text"),
        "{answer}"
    );
    let by_ref = vw_browser::parse(&json!({"action":"click","ref":square["ref"]})).unwrap();
    rig.browser
        .act(SESSION, by_ref, Actor::Agent, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(rig.engine.eval("window.hits").await, 1);
    let at = vw_browser::parse(&json!({"action":"click","ref":square["ref"],
        "x": rect[0] + rect[2] / 2, "y": rect[1] + rect[3] / 2}))
    .unwrap();
    let receipt = rig
        .browser
        .act(SESSION, at, Actor::Agent, &NoNavigation)
        .await;
    assert!(
        receipt.is_ok(),
        "coordinate click from the answer's box: {receipt:?}"
    );
    assert_eq!(rig.engine.eval("window.hits").await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn screenshot_read_labels_its_refs_and_names_the_viewport() {
    let rig = page(SHOP).await;
    let (answer, shot) = rig
        .browser
        .read(SESSION, ReadOptions::default(), true, Actor::Agent)
        .await
        .unwrap();
    assert!(shot.is_some());
    assert_eq!(
        answer["untrustedPageContent"]["viewport"],
        json!({"width": 1280, "height": 800, "unit": "CSS pixels"}),
        "{answer}"
    );
    for (index, r) in refs(&answer).iter().enumerate() {
        assert_eq!(r["label"], json!((index + 1).to_string()), "{r}");
    }
    let plain = agent_read(&rig).await;
    assert!(
        plain["untrustedPageContent"].get("viewport").is_none(),
        "{plain}"
    );
}
