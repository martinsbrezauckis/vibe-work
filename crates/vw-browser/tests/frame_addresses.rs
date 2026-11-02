//! A frame the engine cannot name a native address for yet (created, not yet committed to its document) does not fail
//! the page's read: a child frame is left out of that observation (no refs or text from it), and a main frame is read
//! once it has its address, within a bounded wait. The page-reported address never stands in for the native one.
//! Uses the shared page engine with its frame answers' native addresses withheld.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

async fn framed() -> Rig {
    let rig = Rig::new().await;
    rig.engine
        .eval("document.body.innerHTML = '<button id=\"main\" style=\"position:fixed;left:8px;top:8px\">Main button</button>'; scrollTo(0,0)")
        .await;
    rig.engine.eval("new Promise(resolve=>{const f=document.createElement('iframe');f.style='position:fixed;left:8px;top:60px;width:240px;height:100px';f.onload=resolve;f.srcdoc='<button>Framed button</button><p>Framed text</p>';document.body.append(f)})").await;
    rig
}

fn names(answer: &serde_json::Value) -> Vec<String> {
    answer["untrustedPageContent"]["refs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["name"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_child_frame_without_a_native_address_is_left_out_of_the_read() {
    let rig = framed().await;
    let (answer, _) = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .unwrap();
    assert!(
        names(&answer).iter().any(|n| n == "Framed button"),
        "the child frame is read when it has its address: {answer}"
    );
    rig.engine
        .unaddressed_children
        .store(true, Ordering::Relaxed);
    let (answer, _) = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .expect("a child frame without a native address failed the whole read");
    let names = names(&answer);
    assert!(names.iter().any(|n| n == "Main button"), "{answer}");
    assert!(
        !names.iter().any(|n| n == "Framed button"),
        "a frame without a native address gave refs: {answer}"
    );
    assert!(!answer.to_string().contains("Framed text"), "{answer}");
    // The host's own observation (the refs actions and vault fills resolve) holds nothing of that frame either.
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    assert!(
        model.refs.iter().all(|r| r
            .frame_id
            .as_ref()
            .is_some_and(|f| f.url.starts_with("http"))
            && r.name != "Framed button"),
        "the observation kept a ref of the unaddressed frame: {:?}",
        model.refs
    );
    assert!(!model.hidden_text.contains("Framed text"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_main_frame_without_a_native_address_is_read_once_it_has_one() {
    let rig = framed().await;
    rig.engine.unaddressed_main.store(2, Ordering::Relaxed);
    let (answer, _) = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .expect("a main frame between documents failed the read instead of waiting");
    assert_eq!(rig.engine.unaddressed_main.load(Ordering::Relaxed), 0);
    let names = names(&answer);
    assert!(names.iter().any(|n| n == "Main button"), "{answer}");
    assert!(names.iter().any(|n| n == "Framed button"), "{answer}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_main_frame_that_never_gets_a_native_address_is_reported_as_loading() {
    let rig = framed().await;
    rig.engine
        .unaddressed_main
        .store(u64::MAX, Ordering::Relaxed);
    let start = Instant::now();
    let error = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .expect_err("a page without a native address was read");
    assert!(
        start.elapsed() < Duration::from_secs(12),
        "{:?}",
        start.elapsed()
    );
    assert!(error.contains("still loading"), "{error}");
}
