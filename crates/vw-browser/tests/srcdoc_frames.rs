//! Inline (`srcdoc`) and script-written frames are read and acted on like any frame, with the parent document's origin
//! as theirs; a sandboxed one has an opaque origin (readable, never a vault target). Every other frame kind, and a
//! frame that cannot be read at the moment (between documents, removed, too deep), is left out with its reason and
//! never fails the whole read. Uses the shared page engine in the page world and in an isolated world.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

const PAGE_ORIGIN: &str = "https://93.184.215.14";

async fn page(isolated: bool, script: &str) -> Rig {
    let rig = Rig::new().await;
    rig.engine
        .isolate_scripts
        .store(isolated, Ordering::Relaxed);
    rig.engine
        .eval("document.body.innerHTML = '<button style=\"position:fixed;left:4px;top:4px\">Main button</button>'; scrollTo(0,0)")
        .await;
    rig.engine.eval(script).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    rig
}

async fn read(rig: &Rig) -> serde_json::Value {
    rig.browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .unwrap()
        .0
}

fn refs(answer: &serde_json::Value) -> Vec<serde_json::Value> {
    answer["untrustedPageContent"]["refs"]
        .as_array()
        .unwrap()
        .clone()
}

fn named<'a>(refs: &'a [serde_json::Value], name: &str) -> Option<&'a serde_json::Value> {
    refs.iter().find(|r| r["name"] == name)
}

fn unread(answer: &serde_json::Value) -> Vec<String> {
    answer["untrustedPageContent"]["unreadFrames"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|r| r.as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

const SRCDOC_LOGIN: &str = "new Promise(resolve=>{const f=document.createElement('iframe');f.style='position:fixed;left:4px;top:40px;width:280px;height:150px';f.onload=resolve;f.srcdoc='<form method=post><label>User <input autocomplete=username></label><label>Password <input type=password></label><button>Inline sign in</button></form><p>Inline text</p>';document.body.append(f)})";

#[tokio::test(flavor = "multi_thread")]
async fn a_srcdoc_frame_is_read_and_filled_with_its_parents_origin() {
    for isolated in [false, true] {
        let rig = page(isolated, SRCDOC_LOGIN).await;
        let answer = read(&rig).await;
        let refs = refs(&answer);
        let button = named(&refs, "Inline sign in").unwrap_or_else(|| panic!("{answer}"));
        assert_eq!(button["frameOrigin"], PAGE_ORIGIN, "isolated={isolated}");
        assert!(answer.to_string().contains("Inline text"), "{answer}");
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let password = model
            .refs
            .iter()
            .find(|r| r.login.as_deref() == Some("password"))
            .unwrap();
        let target = rig
            .browser
            .fill_check(SESSION, &password.id, Actor::User)
            .await
            .unwrap_or_else(|e| panic!("isolated={isolated}: {}", e.text));
        assert_eq!(target.origin, PAGE_ORIGIN);
        rig.browser
            .fill_write(SESSION, &target, "fixture-password", Actor::User)
            .await
            .unwrap_or_else(|e| panic!("isolated={isolated}: {}", e.text));
        let typed = rig
            .engine
            .eval("document.querySelector('iframe').contentDocument.querySelector('input[type=password]').value")
            .await;
        assert_eq!(typed, "fixture-password");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nested_srcdoc_inherits_up_the_chain_and_a_cross_origin_child_keeps_its_own() {
    for isolated in [false, true] {
        let rig = page(isolated, "new Promise(resolve=>{const f=document.createElement('iframe');f.style='position:fixed;left:4px;top:40px;width:290px;height:150px';f.srcdoc=`<button>Outer inline</button><iframe srcdoc=\"<button>Inner inline</button>\"></iframe><iframe src=\"https://frame.example.test/login\"></iframe>`;f.onload=()=>setTimeout(resolve,400);document.body.append(f)})").await;
        let answer = read(&rig).await;
        let refs = refs(&answer);
        for name in ["Outer inline", "Inner inline"] {
            let item = named(&refs, name).unwrap_or_else(|| panic!("{name}: {answer}"));
            assert_eq!(
                item["frameOrigin"], PAGE_ORIGIN,
                "{name} isolated={isolated}"
            );
        }
        let cross = named(&refs, "Fixture checkbox").unwrap_or_else(|| panic!("{answer}"));
        assert_eq!(cross["frameOrigin"], "https://frame.example.test");
        let nested = named(&refs, "Nested button").unwrap_or_else(|| panic!("{answer}"));
        assert_eq!(nested["frameOrigin"], "https://nested.example.test");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sandboxed_srcdoc_is_readable_but_never_a_vault_target() {
    for isolated in [false, true] {
        let rig = page(isolated, "new Promise(resolve=>{const f=document.createElement('iframe');f.sandbox='allow-scripts allow-forms';f.style='position:fixed;left:4px;top:40px;width:280px;height:150px';f.onload=resolve;f.srcdoc='<form method=post><input type=password aria-label=Secret><button>Sandboxed sign in</button></form>';document.body.append(f)})").await;
        let answer = read(&rig).await;
        let refs = refs(&answer);
        let button = named(&refs, "Sandboxed sign in").unwrap_or_else(|| panic!("{answer}"));
        assert_eq!(button["frameOrigin"], "null", "isolated={isolated}");
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let password = model
            .refs
            .iter()
            .find(|r| r.login.as_deref() == Some("password"))
            .unwrap();
        let refusal = rig
            .browser
            .fill_check(SESSION, &password.id, Actor::User)
            .await
            .expect_err("an opaque origin was given a site login");
        assert!(
            refusal.text.contains("no verified web origin"),
            "{}",
            refusal.text
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_frame_written_by_its_parents_script_uses_the_parents_origin() {
    for isolated in [false, true] {
        let rig = page(isolated, "const f=document.createElement('iframe');f.style='position:fixed;left:4px;top:40px;width:280px;height:70px';document.body.append(f);f.contentDocument.body.innerHTML='<button>Written button</button>';const g=document.createElement('iframe');g.style='position:fixed;left:4px;top:120px;width:280px;height:70px';document.body.append(g);g.contentDocument.open();g.contentDocument.write('<button>Document written</button>');g.contentDocument.close()").await;
        let answer = read(&rig).await;
        let refs = refs(&answer);
        for name in ["Written button", "Document written"] {
            let item = named(&refs, name).unwrap_or_else(|| panic!("{name}: {answer}"));
            assert_eq!(
                item["frameOrigin"], PAGE_ORIGIN,
                "{name} isolated={isolated}"
            );
        }
    }
}

const DATA_AND_BLOB: &str = "new Promise(resolve=>{const login='<form method=post><label>Password <input type=password></label><button>SEND</button></form>';const d=document.createElement('iframe');d.src='data:text/html,'+encodeURIComponent(login.replace('SEND','Data sign in'));document.body.append(d);const b=document.createElement('iframe');b.src=URL.createObjectURL(new Blob([login.replace('SEND','Blob sign in')],{type:'text/html'}));b.onload=()=>setTimeout(resolve,300);document.body.append(b)})";

#[tokio::test(flavor = "multi_thread")]
async fn data_and_blob_frames_are_read_a_data_frame_without_an_origin() {
    for isolated in [false, true] {
        let rig = page(isolated, DATA_AND_BLOB).await;
        let answer = read(&rig).await;
        let refs = refs(&answer);
        let data = named(&refs, "Data sign in").unwrap_or_else(|| panic!("{answer}"));
        assert_eq!(data["frameOrigin"], "null", "isolated={isolated}");
        // A blob document has the origin of the document that created it.
        let blob = named(&refs, "Blob sign in").unwrap_or_else(|| panic!("{answer}"));
        assert_eq!(blob["frameOrigin"], PAGE_ORIGIN, "isolated={isolated}");
        assert!(unread(&answer).is_empty(), "{answer}");
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let field = |origin: &str| {
            model
                .refs
                .iter()
                .find(|r| {
                    r.login.as_deref() == Some("password")
                        && r.frame_id.as_ref().is_some_and(|f| f.origin == origin)
                })
                .map_or_else(
                    || panic!("no password field with origin {origin}"),
                    |r| r.id.clone(),
                )
        };
        let refusal = rig
            .browser
            .fill_check(SESSION, &field("null"), Actor::User)
            .await
            .expect_err("a data: frame was given a site login");
        assert!(
            refusal.text.contains("no verified web origin"),
            "{}",
            refusal.text
        );
        let target = rig
            .browser
            .fill_check(SESSION, &field(PAGE_ORIGIN), Actor::User)
            .await
            .unwrap_or_else(|e| panic!("isolated={isolated}: {}", e.text));
        assert_eq!(target.origin, PAGE_ORIGIN);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_frame_s_origin_is_its_address_never_what_the_page_says() {
    let rig = page(false, DATA_AND_BLOB).await;
    let mount = rig.service.page_mount(SESSION).unwrap();
    rig.service.evaluate_frames_on_mount(&mount, None,
        "if(location.protocol==='blob:')Object.defineProperty(window,'origin',{value:'https://frame.example.test'});JSON.stringify(true)".into(), None).await.unwrap();
    let answer = read(&rig).await;
    assert!(
        refs(&answer)
            .iter()
            .all(|r| r["frameOrigin"] != "https://frame.example.test"),
        "{answer}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_child_frame_between_documents_is_observed_again() {
    let rig = page(false, SRCDOC_LOGIN).await;
    rig.engine.empty_children.store(2, Ordering::Relaxed);
    let answer = read(&rig).await;
    assert_eq!(rig.engine.empty_children.load(Ordering::Relaxed), 0);
    assert!(
        named(&refs(&answer), "Inline sign in").is_some(),
        "{answer}"
    );
    assert!(unread(&answer).is_empty(), "{answer}");
}

/// Its answer names no frame path, so it accounts for no child window of the page: the page's frames are also named
/// unreadable (a checked click is refused before input until the frame settles).
#[tokio::test(flavor = "multi_thread")]
async fn a_child_frame_that_stays_between_documents_is_left_out_after_the_wait() {
    let rig = page(false, SRCDOC_LOGIN).await;
    rig.engine.empty_children.store(u64::MAX, Ordering::Relaxed);
    let start = Instant::now();
    let answer = read(&rig).await;
    // A child frame gets 2 s of the wait (the main frame keeps the whole page-swap wait).
    assert!(
        start.elapsed() < Duration::from_millis(3500),
        "{:?}",
        start.elapsed()
    );
    assert!(named(&refs(&answer), "Main button").is_some(), "{answer}");
    assert!(
        named(&refs(&answer), "Inline sign in").is_none(),
        "{answer}"
    );
    assert_eq!(
        unread(&answer),
        [
            "a frame was changing documents while it was read",
            "a frame could not be read"
        ],
        "{answer}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_frame_removed_during_the_read_does_not_fail_it() {
    let rig = page(false, SRCDOC_LOGIN).await;
    rig.engine.detached_during_read.store(2, Ordering::Relaxed);
    let answer = read(&rig).await;
    assert_eq!(rig.engine.detached_during_read.load(Ordering::Relaxed), 0);
    assert!(
        named(&refs(&answer), "Inline sign in").is_some(),
        "{answer}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn frames_nested_too_deep_are_left_out_without_failing_the_read() {
    for isolated in [false, true] {
        let rig = page(isolated, "new Promise(resolve=>{let html='<button>Deepest</button>';for(let i=0;i<9;i++)html='<iframe srcdoc=\"'+html.replace(/&/g,'&amp;').replace(/\"/g,'&quot;')+'\"></iframe>';const holder=document.createElement('div');holder.innerHTML=html;document.body.append(holder);const deepest=()=>{let w=window;for(let i=0;i<9;i++){if(!w.frames.length)return false;w=w.frames[0];}try{return !!w.document.querySelector('button')}catch(e){return false}};const wait=()=>deepest()?resolve():setTimeout(wait,50);wait()})").await;
        let answer = read(&rig).await;
        let refs = refs(&answer);
        assert!(named(&refs, "Main button").is_some(), "{answer}");
        assert!(named(&refs, "Deepest").is_none(), "{answer}");
        assert!(
            unread(&answer).contains(&"a frame could not be read".to_owned()),
            "isolated={isolated}: {answer}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_main_frame_on_an_inline_document_address_is_never_read() {
    let rig = page(false, "").await;
    rig.engine.main_srcdoc.store(true, Ordering::Relaxed);
    let error = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .expect_err("a top-level about:srcdoc document was read");
    assert!(error.contains("inline frame document"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn old_handles_of_the_same_frame_neither_collide_nor_delay_the_read() {
    let rig = page(true, SRCDOC_LOGIN).await;
    rig.engine.duplicate_children.store(true, Ordering::Relaxed);
    let start = Instant::now();
    let answer = read(&rig).await;
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "{:?}",
        start.elapsed()
    );
    let refs = refs(&answer);
    assert_eq!(
        refs.iter()
            .filter(|r| r["name"] == "Inline sign in")
            .count(),
        1,
        "{answer}"
    );
    assert_eq!(
        named(&refs, "Inline sign in").unwrap()["frameOrigin"],
        PAGE_ORIGIN
    );
    assert!(unread(&answer).is_empty(), "{answer}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_focused_read_also_names_the_frames_it_left_out() {
    let rig = page(false, SRCDOC_LOGIN).await;
    rig.engine
        .silent_inline_children
        .store(true, Ordering::Relaxed);
    let answer = rig
        .browser
        .read(
            SESSION,
            ReadOptions {
                find: Some("button".into()),
                ..ReadOptions::default()
            },
            false,
            Actor::Agent,
        )
        .await
        .unwrap()
        .0;
    assert!(named(&refs(&answer), "Main button").is_some(), "{answer}");
    assert_eq!(unread(&answer), ["a frame could not be read"], "{answer}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_frame_no_handle_answers_for_is_named_as_left_out() {
    let rig = page(true, SRCDOC_LOGIN).await;
    rig.engine
        .silent_inline_children
        .store(true, Ordering::Relaxed);
    let answer = read(&rig).await;
    assert!(named(&refs(&answer), "Main button").is_some(), "{answer}");
    assert!(
        named(&refs(&answer), "Inline sign in").is_none(),
        "{answer}"
    );
    assert_eq!(unread(&answer), ["a frame could not be read"], "{answer}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_main_frame_without_an_address_keeps_the_whole_page_wait() {
    let rig = page(false, "").await;
    rig.engine
        .unaddressed_main
        .store(u64::MAX, Ordering::Relaxed);
    let start = Instant::now();
    let error = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .expect_err("a page without an address was read");
    assert!(error.contains("still loading"), "{error}");
    assert!(
        start.elapsed() >= Duration::from_secs(8),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_main_frame_whose_native_address_moved_on_is_observed_again() {
    // A link to another site: the main frame's native address moves on before its old document stops answering
    // (a click that navigated was answered "Not run: frame address differs from its native origin").
    let rig = page(false, "").await;
    *rig.engine.main_address.lock().unwrap() = Some("https://93.184.215.15/next".into());
    let engine = rig.engine.clone();
    let settled = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        *engine.main_address.lock().unwrap() = None;
    });
    let answer = read(&rig).await;
    settled.await.unwrap();
    assert!(named(&refs(&answer), "Main button").is_some(), "{answer}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_main_frame_on_another_origin_than_its_native_address_is_never_read() {
    let rig = page(false, "").await;
    *rig.engine.main_address.lock().unwrap() = Some("https://93.184.215.15/next".into());
    let start = Instant::now();
    let error = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .expect_err("a document on another origin than its frame was read");
    assert_eq!(error, "frame address differs from its native origin");
    assert!(
        start.elapsed() >= Duration::from_secs(8),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_main_frame_on_a_data_or_blob_address_is_never_read() {
    for address in ["data:text/html,x", "blob:https://93.184.215.14/0f1e"] {
        let rig = page(false, "").await;
        *rig.engine.main_address.lock().unwrap() = Some(address.into());
        let error = rig
            .browser
            .read(SESSION, ReadOptions::default(), false, Actor::Agent)
            .await
            .expect_err("a top-level data: or blob: document was read");
        assert!(!error.contains("still loading"), "{address}: {error}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_data_frame_is_read_when_its_native_address_is_percent_encoded() {
    let rig = page(true, "new Promise(resolve=>{const d=document.createElement('iframe');d.src='data:text/html,<button>Data button</button>';d.onload=()=>setTimeout(resolve,200);document.body.append(d)})").await;
    rig.engine
        .encoded_data_addresses
        .store(true, Ordering::Relaxed);
    let answer = read(&rig).await;
    let refs = refs(&answer);
    let data = named(&refs, "Data button").unwrap_or_else(|| panic!("{answer}"));
    assert_eq!(data["frameOrigin"], "null");
    assert!(unread(&answer).is_empty(), "{answer}");
}

const SCRIPTLESS: &str = "new Promise(resolve=>{const f=document.createElement('iframe');f.sandbox='';f.style='position:fixed;left:4px;top:40px;width:200px;height:60px';f.srcdoc='<button>Scriptless sandbox</button>';f.onload=()=>setTimeout(resolve,200);document.body.append(f)})";

#[tokio::test(flavor = "multi_thread")]
async fn a_scriptless_sandbox_never_fails_a_read_and_is_named_when_left_out() {
    let rig = page(true, SCRIPTLESS).await;
    let answer = read(&rig).await;
    let refs = refs(&answer);
    assert!(named(&refs, "Main button").is_some(), "{answer}");
    match named(&refs, "Scriptless sandbox") {
        Some(item) => assert_eq!(item["frameOrigin"], "null", "{answer}"),
        None => assert_eq!(unread(&answer), ["a frame could not be read"], "{answer}"),
    }
    println!(
        "MEASURE scriptless sandbox cdp={}: read={} unread={:?}",
        std::env::var("VW_TEST_CDP_FRAMES").unwrap_or_default(),
        named(&refs, "Scriptless sandbox").is_some(),
        unread(&answer)
    );
}
