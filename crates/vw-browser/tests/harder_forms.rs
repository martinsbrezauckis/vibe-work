//! Harder form input and observed state through the production driver on a disposable real DOM.
//! Covers multi-select, rich text, file clearing and validation without network submissions.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}
async fn act(rig: &Rig, name: &str, mut args: Value) -> Result<Value, String> {
    let (model, _, _) = rig
        .browser
        .observe(
            SESSION,
            ReadOptions {
                find: Some(name.into()),
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    args["ref"] = json!(model.refs.iter().find(|r| r.name == name).unwrap().id);
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&args)?,
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .map_err(|e| e.text)
}

#[tokio::test]
async fn native_multiple_select_replaces_all_choices_and_verifies_the_set() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<label>Colors<select multiple aria-label=Colors><option value=r>Red</option><option value=g>Green</option><option value=b>Blue</option></select></label>';window.changes=0;document.querySelector('select').onchange=()=>changes++").await;
    let receipt = act(
        &rig,
        "Colors",
        json!({"action":"select","values":["r","Blue"]}),
    )
    .await
    .unwrap();
    assert_eq!(
        receipt["untrustedPageContent"]["target"]["valueNow"], "Red, Blue",
        "{receipt}"
    );
    assert_eq!(
        rig.engine
            .eval("Array.from(document.querySelector('select').selectedOptions,o=>o.value)")
            .await,
        json!(["r", "b"])
    );
    assert_eq!(rig.engine.eval("changes").await, 1);
    act(&rig, "Colors", json!({"action":"select","values":[]}))
        .await
        .unwrap();
    assert_eq!(
        rig.engine
            .eval("document.querySelector('select').selectedOptions.length")
            .await,
        0
    );
}

#[tokio::test]
async fn rich_editor_readback_contains_the_current_rendered_text() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<div contenteditable=true role=textbox aria-label=Message>Before</div>';window.edits=0;document.querySelector('[contenteditable]').oninput=()=>edits++").await;
    act(&rig, "Message", json!({"action":"type","text":"After"}))
        .await
        .unwrap();
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let value = model
        .refs
        .iter()
        .find(|r| r.name == "Message")
        .unwrap()
        .value
        .as_deref();
    assert_eq!(value, Some("After"));
    assert_eq!(
        rig.engine.eval("edits").await,
        1,
        "one edit event, not a duplicate framework update"
    );
}

#[tokio::test]
async fn paragraph_editor_preserves_intentional_blank_lines_and_rejects_rewrites() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<div contenteditable=true role=textbox aria-label=Message><p>Before</p></div>';document.querySelector('[contenteditable]').oninput=e=>{const lines=window.expected.split('\\n');e.target.replaceChildren(...lines.map(text=>{const p=document.createElement('p');if(text)p.textContent=text;else p.append(document.createElement('br'));return p}))}").await;
    for text in ["After", "First\nSecond\n\nLast", "First\n", "\nFirst", ""] {
        rig.engine
            .eval(&format!("window.expected={}", json!(text)))
            .await;
        let receipt = act(&rig, "Message", json!({"action":"type","text":text})).await;
        assert!(
            receipt.is_ok(),
            "{text:?}: {receipt:?}; DOM {}",
            rig.engine
                .eval("document.querySelector('[contenteditable]').outerHTML")
                .await
        );
        let receipt = receipt.unwrap();
        assert_eq!(receipt["ok"], true, "{text:?}: {receipt}");
    }
    rig.engine.eval("document.querySelector('[contenteditable]').oninput=e=>e.target.innerHTML='<p>Rejected</p>'").await;
    assert!(
        act(&rig, "Message", json!({"action":"type","text":"Wanted"}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn empty_upload_paths_clear_selected_files_and_report_the_live_result() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<label>Attachment<input type=file></label>';const dt=new DataTransfer();dt.items.add(new File(['x'],'old.txt'));document.querySelector('input').files=dt.files;window.changes=0;document.querySelector('input').onchange=()=>changes++").await;
    let receipt = act(&rig, "Attachment", json!({"action":"upload","paths":[]}))
        .await
        .unwrap();
    assert_eq!(
        rig.engine
            .eval("document.querySelector('input').files.length")
            .await,
        0
    );
    assert_eq!(rig.engine.eval("changes").await, 1);
    assert_eq!(receipt["ok"], true, "{receipt}");
    assert_eq!(receipt["result"]["files"], json!([]));
}

#[tokio::test]
async fn validation_is_observed_without_showing_native_validation_ui() {
    let rig = Rig::new().await;
    rig.engine
        .eval(
            "document.body.innerHTML='<label>Email<input type=email required value=wrong></label>'",
        )
        .await;
    let (answer, _) = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .unwrap();
    let field = &answer["untrustedPageContent"]["refs"][0];
    assert_eq!(field["valid"], false, "{answer}");
    assert_ne!(field["validationMessage"].as_str().unwrap(), "");
}

#[tokio::test]
async fn enter_reports_native_validation_instead_of_claiming_a_submission() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<form><input aria-label=Email type=email required value=wrong><button>Send</button></form>';window.sends=0;document.querySelector('form').onsubmit=e=>{e.preventDefault();sends++}").await;
    let receipt = act(&rig, "Email", json!({"action":"press","key":"Enter"}))
        .await
        .unwrap();
    assert!(
        receipt["result"]["effect"]
            .as_str()
            .unwrap()
            .contains("validation blocked"),
        "{receipt}"
    );
    assert_eq!(rig.engine.eval("sends").await, 0);
    act(
        &rig,
        "Email",
        json!({"action":"type","text":"demo@example.test","submit":true}),
    )
    .await
    .unwrap();
    assert_eq!(rig.engine.eval("sends").await, 1);
    rig.engine.eval("document.querySelector('input').value='wrong';document.querySelector('button').formNoValidate=true").await;
    act(&rig, "Email", json!({"action":"press","key":"Enter"}))
        .await
        .unwrap();
    assert_eq!(
        rig.engine.eval("sends").await,
        2,
        "the submitter's explicit no-validation setting is honored"
    );
}

#[tokio::test]
async fn custom_option_readback_tracks_aria_selection() {
    let rig = Rig::new().await;
    rig.engine.eval(r"document.body.innerHTML='<button role=option aria-selected=false>Green</button>'; document.querySelector('button').onclick=e=>e.currentTarget.setAttribute('aria-selected','true');scrollTo(0,0)").await;
    let receipt = act(&rig, "Green", json!({"action":"click"})).await.unwrap();
    assert!(
        receipt["untrustedPageContent"]["updatedRefs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["selected"] == true),
        "{receipt}"
    );
}

#[tokio::test]
async fn image_submit_uses_its_alternative_text_and_reads_back_submission() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<form><input type=image alt=Send width=80 height=30></form>';window.sent=0;document.querySelector('form').onsubmit=e=>{e.preventDefault();sent++}").await;
    let receipt = act(&rig, "Send", json!({"action":"click"})).await.unwrap();
    assert_eq!(rig.engine.eval("sent").await, 1, "{receipt}");
    assert_eq!(receipt["ok"], true, "{receipt}");
}

#[tokio::test]
async fn form_batch_reads_each_value_and_stops_after_a_reverted_field() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<input aria-label=Name><select aria-label=Color><option>Red</option><option>Blue</option></select><input type=checkbox aria-label=Agree><input aria-label=Blocked><input aria-label=Remaining>';document.querySelector('[aria-label=Blocked]').oninput=e=>e.target.value='Rejected'").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = |name: &str| {
        model
            .refs
            .iter()
            .find(|item| item.name == name)
            .unwrap()
            .id
            .clone()
    };
    let args = json!({"fields":[
        {"action":"type","ref":reference("Name"),"text":"Ada"},
        {"action":"select","ref":reference("Color"),"value":"Blue"},
        {"action":"check","ref":reference("Agree"),"checked":true},
        {"action":"type","ref":reference("Blocked"),"text":"Wanted"},
        {"action":"type","ref":reference("Remaining"),"text":"Must not run"}
    ]});
    let receipt = rig
        .browser
        .fill_form(SESSION, &args, Actor::Agent, &NoNavigation, true)
        .await
        .unwrap();
    assert_eq!(receipt["ok"], false, "{receipt}");
    assert_eq!(receipt["completed"], 3, "{receipt}");
    assert_eq!(receipt["failedAt"], 3, "{receipt}");
    let fields = &receipt["untrustedPageContent"]["fields"];
    assert_eq!(fields[0]["target"]["valueNow"], "Ada", "{receipt}");
    assert_eq!(fields[1]["target"]["valueNow"], "Blue", "{receipt}");
    assert_eq!(fields[2]["result"]["checked"], true, "{receipt}");
    assert_eq!(
        rig.engine
            .eval("document.querySelector('[aria-label=Remaining]').value")
            .await,
        ""
    );
    let malformed = json!({"fields":[{"action":"type","ref":reference("Name"),"text":"Changed"},{"action":"press","ref":reference("Name"),"key":"Enter"}]});
    assert!(
        rig.browser
            .fill_form(SESSION, &malformed, Actor::Agent, &NoNavigation, true)
            .await
            .is_err()
    );
    assert_eq!(
        rig.engine
            .eval("document.querySelector('[aria-label=Name]').value")
            .await,
        "Ada"
    );
    let success = json!({"fields":[{"action":"type","ref":reference("Name"),"text":"Grace"}]});
    let receipt = rig
        .browser
        .fill_form(SESSION, &success, Actor::Agent, &NoNavigation, true)
        .await
        .unwrap();
    assert_eq!(receipt["ok"], true, "{receipt}");
    assert_eq!(receipt["completed"], 1, "{receipt}");
    assert_eq!(
        receipt["untrustedPageContent"]["fields"][0]["target"]["valueNow"], "Grace",
        "{receipt}"
    );
}
