//! Browser feedback regression using a real Chromium page and a host marker recorder.
//! The normal driver supplies the point; drawing must never add DOM nodes or page input.
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

#[tokio::test]
async fn successful_click_still_marks_the_same_node_after_its_label_changes() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<button>Mark receipt</button>';document.querySelector('button').onclick=function(){this.textContent='Receipt marked'}").await;
    let mut events = rig.service.action_tracker().subscribe();
    let (before, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let button = before
        .refs
        .iter()
        .find(|r| r.name == "Mark receipt")
        .unwrap();
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"click","ref":button.id})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    assert_eq!(
        rig.engine
            .eval("document.querySelector('button').textContent")
            .await,
        "Receipt marked"
    );
    let (after, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let changed = after
        .refs
        .iter()
        .find(|r| r.name == "Receipt marked")
        .unwrap();
    assert_eq!(changed.node, button.node, "the clicked DOM node survived");
    assert_ne!(
        changed.id, button.id,
        "the reference changed with its label"
    );
    assert_eq!(
        rig.engine.input_markers.lock().unwrap().len(),
        1,
        "completed click needs visible feedback after a label change"
    );
    assert_eq!(
        events.try_recv().unwrap().kind,
        vw_preview::action_tracking::ActionKind::Click
    );
}

#[tokio::test]
async fn typing_feedback_is_permanent_without_changing_the_page() {
    let rig = Rig::new().await;
    let mut events = rig.service.action_tracker().subscribe();
    rig.engine.eval("document.body.innerHTML='<label style=\"position:absolute;top:500px;left:20px\">Message<input id=message></label>';window.extraEvents=0;document.addEventListener('pointerdown',()=>extraEvents++);").await;
    let nodes = rig
        .engine
        .eval("document.querySelectorAll('*').length")
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let field = model.refs.iter().find(|r| r.name == "Message").unwrap();
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"type","ref":field.id,"text":"Hello"})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    let markers = rig.engine.input_markers.lock().unwrap().clone();
    {
        let event = events
            .try_recv()
            .expect("built-in typing must enter the common action timeline");
        assert_eq!(event.kind, vw_preview::action_tracking::ActionKind::Type);
        assert_eq!(
            event.source.browser,
            vw_preview::action_tracking::SourceBrowser::BuiltIn
        );
        assert!(event.geometry.target_box.is_some());
    }
    assert_eq!(markers.len(), 1, "typing feedback is permanent");
    let state = rig.engine.eval("JSON.stringify({nodes:document.querySelectorAll('*').length,events:extraEvents,value:document.querySelector('input').value,box:(()=>{let r=document.querySelector('input').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]})()})").await;
    let state: serde_json::Value = serde_json::from_str(state.as_str().unwrap()).unwrap();
    assert_eq!(state["events"], 0);
    assert_eq!(state["value"], "Hello");
    assert_eq!(state["nodes"], nodes);
    assert!((markers[0].point.0 * 300.0 - state["box"][0].as_f64().unwrap()).abs() < 1.0);
    assert!((markers[0].point.1 * 200.0 - state["box"][1].as_f64().unwrap()).abs() < 1.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn confirmed_native_clicks_share_the_timeline_but_person_input_and_failures_do_not() {
    use vw_preview::action_tracking::{ActionKind, ActionPoint};
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<button style=\"position:fixed;left:20px;top:20px;width:100px;height:40px\">Press</button>';window.clicks=0;document.querySelector('button').onclick=()=>clicks++").await;
    let mut events = rig.service.action_tracker().subscribe();
    for by in [Actor::Agent, Actor::User] {
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let field = model.refs.iter().find(|r| r.name == "Press").unwrap();
        let action = vw_browser::Action::ClickAt {
            reference: field.id.clone(),
            x: 50,
            y: 40,
        };
        rig.browser
            .act(SESSION, action, by, &NoNavigation)
            .await
            .unwrap();
        if by == Actor::Agent {
            let event = events.try_recv().unwrap();
            assert_eq!(event.kind, ActionKind::Click);
            assert_eq!(event.geometry.point, Some(ActionPoint { x: 50.0, y: 40.0 }));
            assert!(event.geometry.target_box.is_some());
            assert_eq!(
                event.source.view_id,
                rig.service.page_mount(SESSION).map(|mount| mount.label)
            );
        }
        assert!(
            events.try_recv().is_err(),
            "the user's click or duplicate feedback entered the timeline"
        );
    }
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let field = model.refs.iter().find(|r| r.name == "Press").unwrap();
    rig.engine
        .eval("document.querySelector('button').remove()")
        .await;
    let result = rig
        .browser
        .act(
            SESSION,
            vw_browser::Action::ClickAt {
                reference: field.id.clone(),
                x: 50,
                y: 40,
            },
            Actor::Agent,
            &NoNavigation,
        )
        .await;
    assert!(result.is_err());
    assert!(
        events.try_recv().is_err(),
        "failed input must not draw a success marker"
    );
    assert_eq!(rig.engine.input_markers.lock().unwrap().len(), 1);
    assert_eq!(rig.engine.eval("clicks").await, 2);
}
