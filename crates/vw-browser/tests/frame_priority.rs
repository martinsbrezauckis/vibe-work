//! Parent-measured frame priority and main-document ref reservations through real Chromium pages.
//! Uses the production browser reader and offline cross-origin fixtures; child DOM cannot claim page coverage.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::json;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions, model::PageModel};

const CHILD: &str = "https://consent.example.test/privacy";

async fn page(style: &str, fields: usize) -> Rig {
    let rig = Rig::new().await;
    rig.engine.viewport(1000, 800).await;
    rig.engine
        .eval(&format!(
            "document.head.innerHTML=''; document.body.style='margin:0';
         document.body.innerHTML='<form>' + Array.from({{length:{fields}}}, (_,i)=>
           '<input aria-label=\"Field '+i+'\">').join('') + '</form>';
         const f=document.createElement('iframe'); f.src={}; f.style={};
         document.body.append(f); scrollTo(0,0)",
            json!(CHILD),
            json!(style)
        ))
        .await;
    rig.engine.wait_for_frame(CHILD, "#reject").await;
    rig
}

async fn flood(rig: &Rig) {
    rig.engine.eval_in_frame_pages(
        "if(window!==top) { document.body.innerHTML=Array.from({length:100},(_,i)=>
         '<button role=\"dialog\" style=\"position:fixed;left:0;top:0\">Ad '+i+'</button>').join(''); }"
    ).await;
}

async fn observe(rig: &Rig, max_refs: usize) -> PageModel {
    rig.browser
        .observe(
            SESSION,
            ReadOptions {
                max_refs,
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap()
        .0
}

fn main_count(model: &PageModel) -> usize {
    model.refs.iter().filter(|r| r.frame != Some(true)).count()
}

#[tokio::test]
async fn small_ad_dialogs_follow_every_main_form_field() {
    let rig = page(
        "position:fixed;right:0;top:0;width:200px;height:120px;border:0",
        30,
    )
    .await;
    flood(&rig).await;
    let model = observe(&rig, 32).await;
    assert_eq!(model.ref_count, 130);
    assert_eq!(
        main_count(&model),
        30,
        "all 30 main fields must survive the 32-ref cap"
    );
    assert!(
        model.refs[..30]
            .iter()
            .all(|r| r.name.starts_with("Field "))
    );
    assert!(
        model.refs[30..]
            .iter()
            .all(|r| r.name.starts_with("Ad ") && r.priority <= 3)
    );
}

#[tokio::test]
async fn page_covering_consent_buttons_remain_first() {
    let rig = page("position:fixed;inset:0;width:100%;height:100%;border:0", 30).await;
    let model = observe(&rig, 32).await;
    assert!(model.refs[..3].iter().all(|r| r.frame == Some(true)
        && r.role == "button"
        && r.priority == 6
        && r.frame_id.as_ref().unwrap().url == CHILD));
    assert_eq!(main_count(&model), 29);
}

#[tokio::test]
async fn large_frame_flood_reserves_half_and_explains_continuation() {
    let rig = page("position:fixed;inset:0;width:100%;height:100%;border:0", 30).await;
    flood(&rig).await;
    for cap in [1usize, 2, 31, 32, 200] {
        let model = observe(&rig, cap).await;
        let reserved = cap.div_ceil(2).min(30);
        assert!(
            main_count(&model) >= reserved,
            "cap {cap}: reserved {reserved}, got {}",
            main_count(&model)
        );
        assert_eq!(model.refs.len(), cap.min(130));
        if cap > 1 {
            assert!(model.refs[0].name.starts_with("Ad "));
        }
    }
    let (answer, _) = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .unwrap();
    let continuation = format!("{} {}", answer["note2"], answer["trimmed"]["how"]);
    assert!(
        continuation.contains("half") && continuation.contains("main document"),
        "{continuation}"
    );
    assert!(continuation.contains("find") && continuation.contains("tab"));
    let refs = answer["untrustedPageContent"]["refs"].as_array().unwrap();
    assert_ne!(refs.as_slice(), &[] as &[serde_json::Value]);
    assert!(
        refs.iter().filter(|r| r["frame"] != true).count() >= refs.len().div_ceil(2),
        "{answer}"
    );
}

#[tokio::test]
async fn coverage_uses_visible_area_and_includes_the_threshold() {
    let rig = page(
        "position:fixed;left:0;top:0;width:1000px;height:319px;border:0",
        30,
    )
    .await;
    flood(&rig).await;
    for (style, promoted) in [
        ("height:319px", false),
        ("height:320px", true),
        ("height:800px;left:800px", false),
        (
            "height:800px;left:0;transform:scale(.5);transform-origin:top left",
            false,
        ),
        (
            "height:800px;left:0;transform:none;visibility:hidden",
            false,
        ),
    ] {
        rig.engine.eval(&format!("document.querySelector('iframe').style.cssText='position:fixed;left:0;top:0;width:1000px;border:0;'+{}", json!(style))).await;
        let model = observe(&rig, 32).await;
        assert_eq!(model.refs[0].name.starts_with("Ad "), promoted, "{style}");
        assert!(main_count(&model) >= 16);
    }
}

#[tokio::test]
async fn nested_full_viewport_cannot_escape_its_small_parent() {
    let rig = page(
        "position:fixed;left:0;top:0;width:200px;height:120px;border:0",
        30,
    )
    .await;
    rig.engine.eval_in_frame_pages(
        "if(window!==top) document.body.innerHTML='<iframe src=\"https://nested.example.test/child\" style=\"position:fixed;left:0;top:0;width:1000px;height:800px;border:0\"></iframe>'"
    ).await;
    rig.engine
        .wait_for_frame("https://nested.example.test/child", "#nested")
        .await;
    rig.engine.eval_in_frame_pages(
        "if(location.hostname==='nested.example.test') document.body.innerHTML=Array.from({length:100},(_,i)=>'<button role=dialog style=\"position:fixed;left:0;top:0\">Nested '+i+'</button>').join('')"
    ).await;
    let model = observe(&rig, 32).await;
    assert_eq!(main_count(&model), 30);
    assert!(
        model.refs[..30]
            .iter()
            .all(|r| r.name.starts_with("Field "))
    );
    assert!(
        model.refs[30..]
            .iter()
            .all(|r| r.name.starts_with("Nested ") && r.priority <= 3)
    );
}

#[tokio::test]
async fn unused_main_reservation_is_available_to_frames() {
    let rig = page("position:fixed;inset:0;width:100%;height:100%;border:0", 0).await;
    flood(&rig).await;
    assert_eq!(observe(&rig, 32).await.refs.len(), 32);
    rig.engine
        .eval("document.querySelector('form').innerHTML='<input aria-label=Field>'")
        .await;
    let model = observe(&rig, 32).await;
    assert_eq!(model.refs.len(), 32);
    assert_eq!(main_count(&model), 1);
}

#[tokio::test]
async fn several_covering_frames_share_one_child_budget() {
    let rig = page("position:fixed;inset:0;width:100%;height:100%;border:0", 30).await;
    rig.engine.eval("const f=document.querySelector('iframe').cloneNode(); f.src='https://consent.example.test/other'; document.body.append(f)").await;
    rig.engine
        .wait_for_frame("https://consent.example.test/other", "#reject")
        .await;
    flood(&rig).await;
    let model = observe(&rig, 32).await;
    assert_eq!(model.ref_count, 230);
    assert_eq!(main_count(&model), 16);
    // Selection precedes the absolute 200-ref model bound as well as the requested cap.
    let model = observe(&rig, 200).await;
    assert_eq!(model.refs.len(), 200);
    assert_eq!(main_count(&model), 30);
}

#[tokio::test]
async fn same_origin_fallback_reserves_main_fields_before_its_script_cap() {
    let rig = page(
        "position:fixed;left:0;top:0;width:200px;height:120px;border:0",
        30,
    )
    .await;
    rig.engine
        .eval("window['fallback-fixture']={stability:new WeakMap()}")
        .await;
    rig.engine.eval("new Promise(resolve=>{const f=document.querySelector('iframe'); f.onload=resolve; f.srcdoc=Array.from({length:100},(_,i)=>'<button role=dialog style=\"position:fixed;left:0;top:0\">Ad '+i+'</button>').join('')})").await;
    for (style, expected) in [
        ("width:200px;height:120px", 30),
        ("width:100%;height:100%", 16),
    ] {
        rig.engine.eval(&format!("document.querySelector('iframe').style.cssText='position:fixed;left:0;top:0;border:0;'+{}", json!(style))).await;
        let raw = rig
            .engine
            .eval(&vw_browser::scripts::observe(
                "fallback-fixture",
                32,
                2000,
                0,
            ))
            .await;
        let model: PageModel = serde_json::from_str(raw.as_str().unwrap()).unwrap();
        assert_eq!(model.refs.len(), 32);
        assert_eq!(main_count(&model), expected, "{style}");
    }
}
