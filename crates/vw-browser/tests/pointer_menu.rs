//! Native pointer variants reach page handlers without opening the host menu. Uses the real Chromium fixture,
//! production isolated page initializer and checked input; the native `WebKit` fixture uses the same HTML.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::json;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

async fn prepared() -> Rig {
    let rig = Rig::new_in_world(true).await;
    let html = serde_json::to_string(include_str!("fixtures/pointer.html")).unwrap();
    rig.engine
        .eval(&format!(
            "document.open();document.write({html});document.close()"
        ))
        .await;
    let mount = rig.service.page_mount(SESSION).unwrap();
    let page = rig.service.page_accessor(SESSION).unwrap();
    let init = vw_preview::page_init_script(&page)
        .replace("const TRUSTED = false;", "const TRUSTED = true;");
    rig.service
        .evaluate_frames_on_mount(&mount, None, format!("{init}\nJSON.stringify(true)"), None)
        .await
        .unwrap();
    rig
}

async fn act(rig: &Rig, args: serde_json::Value) {
    rig.engine
        .eval("document.body.dataset.events = document.body.dataset.pointerEvents = '[]'")
        .await;
    let result = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(&args).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await;
    assert_eq!(
        result.as_ref().unwrap()["result"]["nativeClick"],
        true,
        "{args}: {result:?}"
    );
}

async fn assert_event(rig: &Rig, event: &str, target: &str) {
    let events = rig
        .engine
        .eval("JSON.parse(document.body.dataset.events)")
        .await;
    let sequence = rig
        .engine
        .eval("JSON.parse(document.body.dataset.pointerEvents)")
        .await;
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == event && e["target"] == target && e["trusted"] == true),
        "missing {event} on {target}: {events}; pointer sequence: {sequence}"
    );
    assert_eq!(
        rig.engine.eval("document.body.dataset.appMenus").await,
        "0",
        "agent opened the app menu"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_right_click_reaches_the_page_and_leaves_later_native_input_unblocked() {
    let rig = prepared().await;
    for (action, event) in [
        ("page.right_click", "contextmenu"),
        ("page.double_click", "dblclick"),
        ("page.middle_click", "auxclick"),
        ("page.right_click", "contextmenu"),
    ] {
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let target = model.refs.iter().find(|r| r.name == "Menu button").unwrap();
        act(&rig, json!({"action":action,"ref":target.id})).await;
        assert_event(&rig, event, "target").await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_variants_accept_non_controls_with_pointer_handlers() {
    let rig = prepared().await;
    for (action, event) in [
        ("page.double_click", "dblclick"),
        ("page.right_click", "contextmenu"),
        ("page.middle_click", "auxclick"),
    ] {
        // Each attribute must be observable on its own, with no role, tabindex or pointer cursor.
        let tag = if event == "auxclick" { "span" } else { "div" };
        rig.engine.eval(&format!("document.querySelector('#cell').outerHTML = '<{tag} id=cell on{event}=\"record(event)\">Cell A1</{tag}>'")).await;
        rig.browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        act(&rig, json!({"action":action,"x":50,"y":120})).await;
        assert_event(&rig, event, "cell").await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_middle_click_survives_ref_double_click_in_the_same_document() {
    let rig = prepared().await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    for (action, event, by_ref) in [
        ("page.middle_click", "auxclick", false),
        ("page.double_click", "dblclick", false),
        ("page.middle_click", "auxclick", false),
        ("page.double_click", "dblclick", true),
        ("page.middle_click", "auxclick", false),
        ("page.middle_click", "auxclick", true),
        ("page.middle_click", "auxclick", false),
    ] {
        let (name, node, y) = if event == "auxclick" {
            ("Aux target", "aux", 170)
        } else {
            ("Cell A1", "cell", 120)
        };
        let target = model.refs.iter().find(|r| r.name == name).unwrap();
        let args = if by_ref {
            json!({"action":action,"ref":target.id})
        } else {
            json!({"action":action,"x":50,"y":y})
        };
        act(&rig, args).await;
        assert_event(&rig, event, node).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_middle_click_survives_the_ref_then_coordinate_sequence() {
    let rig = prepared().await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    for by_ref in [true, false] {
        for (action, event, name, node, y) in [
            (
                "page.right_click",
                "contextmenu",
                "Menu button",
                "target",
                40,
            ),
            ("page.double_click", "dblclick", "Cell A1", "cell", 120),
            ("page.middle_click", "auxclick", "Aux target", "aux", 170),
        ] {
            let target = model.refs.iter().find(|r| r.name == name).unwrap();
            let args = if by_ref {
                json!({"action":action,"ref":target.id})
            } else {
                json!({"action":action,"x":50,"y":y})
            };
            act(&rig, args).await;
            assert_event(&rig, event, node).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_middle_click_after_ref_double_click_on_scrollable_page() {
    let rig = prepared().await;
    rig.engine
        .eval("document.body.style.height = '2000px'")
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let target = model.refs.iter().find(|r| r.name == "Cell A1").unwrap();
    act(&rig, json!({"action":"page.double_click","ref":target.id})).await;
    assert_event(&rig, "dblclick", "cell").await;
    act(&rig, json!({"action":"page.middle_click","x":50,"y":170})).await;
    assert_event(&rig, "auxclick", "aux").await;
    rig.engine.eval("document.body.dataset.events = '[]'").await;
    rig.engine.person_middle_click(50.0, 170.0).await;
    assert_eq!(
        rig.engine
            .eval("JSON.parse(document.body.dataset.events)")
            .await,
        json!([]),
        "the collected agent receipt must leave the user's autoscroll default alone"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn checked_middle_click_keeps_the_links_new_tab_default() {
    let rig = prepared().await;
    rig.engine.eval("document.body.style.height = '2000px'; document.querySelector('#aux').outerHTML = '<a id=aux href=https://example.test/middle onauxclick=record(event)>Aux target</a>'").await;
    rig.browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    act(&rig, json!({"action":"page.middle_click","x":50,"y":170})).await;
    assert_event(&rig, "auxclick", "aux").await;
    assert_eq!(
        rig.engine.page_count().await,
        2,
        "middle click must still open the link's new tab"
    );
}
