//! `browser_act upload`: files reach a page's file field as a person's file picker leaves them (the field holds
//! them, the page gets `input` and `change`); one file only for a single-file field; a ref with no file field is
//! refused; a label or a "Choose file" control resolves to its field.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::json;
use support::{Rig, SESSION};
use vw_browser::{Action, Actor, ReadOptions, UploadFile};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

fn file(name: &str, size: usize) -> UploadFile {
    UploadFile {
        name: name.to_owned(),
        mime: "text/plain".to_owned(),
        data: (0..size)
            .map(|i| b'a' + u8::try_from(i % 26).unwrap_or(0))
            .collect(),
    }
}

async fn upload(
    rig: &Rig,
    name: &str,
    files: Vec<UploadFile>,
) -> Result<serde_json::Value, String> {
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = model
        .refs
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no ref named {name}: {model:?}"))
        .id
        .clone();
    let parsed = vw_browser::parse(&json!({"action": "upload", "ref": reference, "paths": ["x"]}))?;
    let Action::Upload {
        reference, paths, ..
    } = parsed
    else {
        panic!("not an upload: {parsed:?}")
    };
    rig.browser
        .act(
            SESSION,
            Action::Upload {
                reference,
                paths,
                files,
            },
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .map_err(|error| error.text)
}

const PAGE: &str = "document.body.innerHTML = `\
<label>Your report <input type=file id=one></label>\
<label for=many>Your photos</label><input type=file id=many multiple>\
<button type=button id=pick onclick=\"document.getElementById('hidden').click()\">Choose file</button>\
<input type=file id=hidden style=display:none>\
<button type=button>Nothing to attach</button><p id=seen></p>`; scrollTo(0,0);\
window.seen = []; for (const f of document.querySelectorAll('input[type=file]')) for (const t of ['input','change']) \
f.addEventListener(t, () => seen.push(t + ':' + f.id + ':' + Array.from(f.files).map(x => x.name + '/' + x.size).join(',')));";

#[tokio::test]
async fn upload_does_not_claim_success_when_the_page_replaces_the_file() {
    let rig = Rig::new().await;
    rig.engine.eval(PAGE).await;
    rig.engine.eval("document.getElementById('one').addEventListener('change',event=>{const data=new DataTransfer();data.items.add(new File(['changed'],'different.txt'));event.target.files=data.files;});").await;
    let result = upload(&rig, "Your report", vec![file("report.txt", 10)]).await;
    assert!(
        result.is_err(),
        "replaced file reported as uploaded: {result:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_sets_the_files_and_the_page_sees_input_and_change() {
    let rig = Rig::new().await;
    rig.engine.eval(PAGE).await;
    // 1.6 MB: more than one chunk.
    let receipt = upload(&rig, "Your report", vec![file("report.txt", 1_600_000)])
        .await
        .unwrap();
    assert_eq!(
        receipt["result"]["files"],
        json!([{"name": "report.txt", "size": 1_600_000}]),
        "{receipt}"
    );
    assert_eq!(
        rig.engine.eval("seen").await,
        json!([
            "input:one:report.txt/1600000",
            "change:one:report.txt/1600000"
        ])
    );
    let content = rig
        .engine
        .eval("(async () => (await document.getElementById('one').files[0].text()).slice(0, 5))()")
        .await;
    assert_eq!(content, json!("abcde"));
}

#[tokio::test(flavor = "multi_thread")]
async fn several_files_only_where_the_field_takes_several() {
    let rig = Rig::new().await;
    rig.engine.eval(PAGE).await;
    let refused = upload(
        &rig,
        "Your report",
        vec![file("a.txt", 3), file("b.txt", 4)],
    )
    .await
    .unwrap_err();
    assert!(refused.contains("takes one file"), "{refused}");
    let receipt = upload(
        &rig,
        "Your photos",
        vec![file("a.txt", 3), file("b.txt", 4)],
    )
    .await
    .unwrap();
    assert_eq!(
        receipt["result"]["files"],
        json!([{"name": "a.txt", "size": 3}, {"name": "b.txt", "size": 4}])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn long_file_names_keep_every_upload_outcome_inline() {
    let rig = Rig::new().await;
    rig.engine.eval(PAGE).await;
    let files = (0..16)
        .map(|i| file(&format!("report-{}-{i}.txt", "é".repeat(100)), i + 1))
        .collect();
    let receipt = upload(&rig, "Your photos", files).await.unwrap();
    assert!(
        receipt.to_string().len() <= 4000,
        "{} bytes",
        receipt.to_string().len()
    );
    assert!(serde_json::to_string_pretty(&receipt).unwrap().len() <= 4500);
    assert_eq!(receipt["result"]["files"].as_array().unwrap().len(), 16);
    assert_eq!(receipt["result"]["fileNamesOmitted"], true);
    assert_eq!(
        rig.engine
            .eval("document.getElementById('many').files.length")
            .await,
        16
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_choose_file_button_resolves_to_the_pages_field_and_other_refs_are_refused() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML = `<button type=button onclick=\"document.getElementById('hidden').click()\">Choose file</button><input type=file id=hidden style=display:none><button type=button>Nothing to attach</button>`; scrollTo(0,0); window.seen = []; document.getElementById('hidden').addEventListener('change', e => seen.push(e.target.files[0].name));").await;
    upload(&rig, "Choose file", vec![file("cv.txt", 10)])
        .await
        .unwrap();
    assert_eq!(rig.engine.eval("seen").await, json!(["cv.txt"]));
    rig.engine
        .eval("document.body.innerHTML = '<button type=button>Nothing to attach</button>'")
        .await;
    let refused = upload(&rig, "Nothing to attach", vec![file("cv.txt", 10)])
        .await
        .unwrap_err();
    assert!(refused.contains("takes no file"), "{refused}");
}

/// The `WebView2` route (`VW_TEST_CDP_FRAMES=1`: Chromium's `DevTools` isolated worlds, as on Windows): the agent's
/// click runs as a user gesture, so the page's file field asks for a picker; interception keeps it from showing and
/// the receipt names the upload action. Plain mode has no gesture (no picker can open there): this test then
/// prints NOT RUN and checks nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_picker_the_agents_click_opens_never_shows_and_the_receipt_says_so() {
    if std::env::var("VW_TEST_CDP_FRAMES").as_deref() != Ok("1") {
        eprintln!("NOT RUN: the picker route needs VW_TEST_CDP_FRAMES=1");
        return;
    }
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML = '<label>Your report <input type=file id=one></label>'; scrollTo(0,0)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let field = model.refs.iter().find(|r| r.name == "Your report").unwrap();
    let receipt = rig
        .browser
        .act(
            SESSION,
            Action::Click {
                reference: field.id.clone(),
            },
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    let note = receipt["filePicker"].as_str().unwrap_or_default();
    assert!(
        note.contains("never opens for the agent")
            && note.contains(&format!("\"ref\": \"{}\"", field.id)),
        "{receipt}"
    );
}

/// The same for the agent's native (x/y) click: the act window is opened for every input, so the click's picker is
/// intercepted too (`VW_TEST_CDP_FRAMES=1`; NOT RUN otherwise).
#[tokio::test(flavor = "multi_thread")]
async fn a_picker_the_agents_native_click_opens_never_shows_either() {
    if std::env::var("VW_TEST_CDP_FRAMES").as_deref() != Ok("1") {
        eprintln!("NOT RUN: the picker route needs VW_TEST_CDP_FRAMES=1");
        return;
    }
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML = '<label>Your report <input type=file id=one style=\"width:200px;height:40px\"></label>'; scrollTo(0,0)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let field = model.refs.iter().find(|r| r.name == "Your report").unwrap();
    let bounds = field.bounds.unwrap();
    let receipt = rig
        .browser
        .act(
            SESSION,
            Action::ClickAt {
                reference: field.id.clone(),
                x: u32::try_from(bounds.x + bounds.width / 2).unwrap(),
                y: u32::try_from(bounds.y + bounds.height / 2).unwrap(),
            },
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    assert_eq!(receipt["result"]["nativeClick"], true, "{receipt}");
    let note = receipt["filePicker"].as_str().unwrap_or_default();
    assert!(note.contains("never opens for the agent"), "{receipt}");
}
