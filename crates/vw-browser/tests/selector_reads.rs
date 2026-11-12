//! Selector counts and indexed refs through the real driver, including document-bound input and frames.
//! Uses the Chromium test engine; selector reads never trigger clicks or expose protected field values.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::json;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions, Selector, SelectorKind};

fn options(value: &str, index: Option<usize>) -> ReadOptions {
    ReadOptions {
        selector: Some(Selector {
            kind: SelectorKind::Css,
            value: value.into(),
            index,
        }),
        ..ReadOptions::default()
    }
}

#[tokio::test]
async fn indexed_matches_keep_real_refs_and_report_total_before_caps() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML=Array.from({length:250},(_,i)=>'<button>Choice '+i+'</button>').join('')").await;
    let (answer, _) = rig
        .browser
        .read(SESSION, options("button", Some(190)), false, Actor::Agent)
        .await
        .unwrap();
    assert_eq!(answer["matches"]["count"], 250, "{answer}");
    assert_eq!(answer["matches"]["complete"], true, "{answer}");
    let refs = answer["untrustedPageContent"]["refs"].as_array().unwrap();
    assert_eq!(refs.len(), 1, "{answer}");
    assert_eq!(refs[0]["name"], "Choice 190");
    assert!(vw_browser::model::is_ref(refs[0]["ref"].as_str().unwrap()));
    assert!(
        rig.browser
            .read(SESSION, options("[", None), false, Actor::Agent)
            .await
            .unwrap_err()
            .contains("syntax")
    );
    assert!(
        rig.browser
            .read(SESSION, options("button", Some(200)), false, Actor::Agent)
            .await
            .unwrap_err()
            .contains("199")
    );
}

#[tokio::test]
async fn frame_matches_are_counted_once_and_follow_the_main_document() {
    let rig = Rig::new().await;
    rig.engine.eval(r#"new Promise(resolve=>{document.body.innerHTML='<input aria-label="Main name">';const f=document.createElement('iframe');f.onload=resolve;f.srcdoc='<input aria-label="Frame name">';document.body.append(f)})"#).await;
    let (answer, _) = rig
        .browser
        .read(SESSION, options("input", Some(1)), false, Actor::Agent)
        .await
        .unwrap();
    assert_eq!(answer["matches"]["count"], 2, "{answer}");
    assert_eq!(
        answer["untrustedPageContent"]["refs"][0]["name"], "Frame name",
        "{answer}"
    );
    assert_eq!(answer["untrustedPageContent"]["refs"][0]["frame"], true);
    let (empty, _) = rig
        .browser
        .read(SESSION, options("#missing", None), false, Actor::Agent)
        .await
        .unwrap();
    assert_eq!(empty["matches"]["count"], 0);
    assert_eq!(empty["untrustedPageContent"]["refs"], json!([]));
}

#[tokio::test]
async fn subtree_read_binds_the_retained_frame_node_and_refuses_replacement() {
    let rig = Rig::new().await;
    rig.engine.eval(r"new Promise(resolve=>{document.body.innerHTML='<button>Outside</button>';const f=document.createElement('iframe');f.onload=resolve;f.srcdoc='<section id=chosen>Inside<button>Keep</button></section><button>Sibling</button>';document.body.append(f)})").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, options("#chosen", None))
        .await
        .unwrap();
    let opts = ReadOptions {
        root_ref: Some(model.refs[0].id.clone()),
        ..ReadOptions::default()
    };
    let (answer, _) = rig
        .browser
        .read(SESSION, opts.clone(), false, Actor::Agent)
        .await
        .unwrap();
    assert_eq!(
        answer["untrustedPageContent"]["refs"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "{answer}"
    );
    assert_eq!(
        answer["untrustedPageContent"]["refs"][0]["name"], "Keep",
        "{answer}"
    );
    assert!(!answer.to_string().contains("Sibling"), "{answer}");
    rig.engine.eval("document.querySelector('iframe').contentDocument.querySelector('#chosen').outerHTML='<section id=chosen><button>Replacement</button></section>'").await;
    assert!(
        rig.browser
            .read(SESSION, opts, false, Actor::Agent)
            .await
            .unwrap_err()
            .contains("subtree ref changed")
    );
}
