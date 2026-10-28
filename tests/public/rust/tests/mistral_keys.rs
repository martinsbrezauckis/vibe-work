//! Multiple model keys through the dispatcher, with disposable vaults and loopback Mistral.
//! Exercises metadata, selection events, confirmation, busy refusal and the vault origin rule.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::core::{ScriptRig, no_vibe_core_with, real_core};
use serde_json::{Value, json};
use vw_core_api::{Command, Core, ErrorCode, Origin};
use vw_gate::{Confirmation, Requester};
use vw_keys::{KeyEvent, KeyService};
use vw_vault::SecretString;

const PERSONAL: &str = "synthetic-personal-model-key";
const WORK: &str = "synthetic-work-model-key";

fn cmd(request: Value) -> Command {
    serde_json::from_value(json!({"type":"keys","request":request})).unwrap()
}

async fn run(core: &Core, request: Value) -> Value {
    serde_json::to_value(core.execute(Origin::Ui, cmd(request)).await.unwrap()).unwrap()
}

async fn rig(name: &str, base: &str) -> (ScriptRig, Arc<KeyService>) {
    rig_with_model(name, base, None).await
}

async fn rig_with_model(
    name: &str,
    base: &str,
    model: Option<common::scripted_model::Script>,
) -> (ScriptRig, Arc<KeyService>) {
    let dir = common::processes::scratch(name);
    let keys = KeyService::start(common::vault::key_service_config(
        &dir,
        base,
        Arc::default(),
    ))
    .await
    .unwrap();
    keys.create_vault(SecretString::new("synthetic passphrase"))
        .await
        .unwrap();
    keys.store_key(SecretString::new(PERSONAL)).await.unwrap();
    let core = if let Some(script) = model {
        real_core(name, &[], None, script, |c| c.keys = Some(keys.clone()))
            .await
            .0
    } else {
        no_vibe_core_with(name, &[], |c| c.keys = Some(keys.clone())).await
    };
    (core, keys)
}

#[tokio::test]
async fn list_add_switch_rename_remove_never_return_values() {
    let fake = common::fake_mistral::start().await;
    let (rig, keys) = rig("multiple-keys", &fake.base()).await;
    keys.store_secret(
        "Unrelated".into(),
        SecretString::new("synthetic-other-secret"),
        None,
    )
    .await
    .unwrap();
    let before = run(&rig.core, json!({"op":"listMistralKeys"})).await;
    let original = before["keys"][0]["itemId"].as_str().unwrap().to_owned();
    let mut events = keys.subscribe();
    run(
        &rig.core,
        json!({"op":"addMistralKey","name":"Work key","apiKey":WORK}),
    )
    .await;
    let listed = run(&rig.core, json!({"op":"listMistralKeys"})).await;
    let rows = listed["keys"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let second = rows.iter().find(|k| k["label"] == "Work key").unwrap();
    let id = second["itemId"].as_str().unwrap();
    assert_ne!(id, original);
    assert_eq!(second["inUse"], false);
    assert_eq!(second["lastTest"]["modelCount"], 2);
    assert!(!listed.to_string().contains(PERSONAL));
    assert!(!listed.to_string().contains(WORK));
    assert!(!listed.to_string().contains("Unrelated"));
    assert_eq!(
        keys.status().await.unwrap().active_key.unwrap().label,
        before["keys"][0]["label"]
    );
    while let Ok(event) = events.try_recv() {
        assert_ne!(event, KeyEvent::KeyChanged);
    }
    rig.confirmer.then(Confirmation::Declined);
    let error = rig
        .core
        .execute(Origin::Ui, cmd(json!({"op":"useMistralKey","itemId":id})))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ConfirmationDeclined);
    assert_eq!(error.message, "The Mistral key was kept.");
    assert_eq!(
        run(&rig.core, json!({"op":"listMistralKeys"})).await,
        listed
    );
    rig.confirmer.then(Confirmation::Approved);
    run(&rig.core, json!({"op":"useMistralKey","itemId":id})).await;
    let changed = std::iter::from_fn(|| events.try_recv().ok())
        .filter(|e| *e == KeyEvent::KeyChanged)
        .count();
    assert_eq!(changed, 1);
    let shown = rig.confirmer.shown();
    assert_eq!(shown.last().unwrap().requester, Requester::Window);
    let summary = &shown.last().unwrap().summary;
    assert_eq!(
        summary,
        "Use the key “Work key” for Mistral? Chats continue with the new key."
    );
    run(
        &rig.core,
        json!({"op":"renameMistralKey","itemId":id,"name":"Company"}),
    )
    .await;
    assert_eq!(
        keys.status().await.unwrap().active_key.unwrap().label,
        "Company"
    );
    let error = rig
        .core
        .execute(
            Origin::Ui,
            cmd(json!({"op":"removeMistralKey","itemId":id})),
        )
        .await
        .unwrap_err();
    assert_eq!(error.server_code.as_deref(), Some("keyInUse"));
    run(&rig.core, json!({"op":"useMistralKey","itemId":original})).await;
    run(&rig.core, json!({"op":"removeMistralKey","itemId":id})).await;
    assert_eq!(
        run(&rig.core, json!({"op":"listMistralKeys"})).await["keys"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    keys.lock();
    assert!(
        rig.core
            .execute(Origin::Ui, cmd(json!({"op":"listMistralKeys"})))
            .await
            .is_err()
    );
    rig.core.shutdown().await;
    keys.shutdown().await;
}

#[tokio::test]
async fn debug_and_agent_cannot_mutate_model_keys() {
    let fake = common::fake_mistral::start().await;
    let (rig, keys) = rig("multiple-key-origins", &fake.base()).await;
    for request in [
        json!({"op":"addMistralKey","name":"Work key","apiKey":WORK}),
        json!({"op":"useMistralKey","itemId":"other"}),
        json!({"op":"renameMistralKey","itemId":"other","name":"Changed"}),
        json!({"op":"removeMistralKey","itemId":"other"}),
    ] {
        let command = cmd(request.clone());
        assert!(!format!("{command:?}").contains(WORK));
        assert!(!serde_json::to_string(&command).unwrap().contains(WORK));
        let error = rig
            .core
            .execute(Origin::DebugApi, command)
            .await
            .unwrap_err();
        assert_eq!(error.server_code.as_deref(), Some("vaultUserOnly"));
        let gate = common::gate::rig();
        let method = format!("keys/{}", request["op"].as_str().unwrap());
        assert!(
            gate.gate
                .request_confirmation_as(&method, &request, Requester::Agent)
                .await
                .is_err()
        );
        assert!(gate.transport.frames().is_empty());
    }
    assert!(
        rig.core
            .execute(Origin::DebugApi, cmd(json!({"op":"listMistralKeys"})))
            .await
            .is_ok()
    );
    assert!(rig.confirmer.shown().is_empty());
    rig.core.shutdown().await;
    keys.shutdown().await;
}

#[tokio::test]
async fn a_working_chat_refuses_before_confirmation() {
    let fake = common::fake_mistral::start().await;
    let script = serde_json::from_value(json!({"rules":[
        {"side":true,"reply":{"text":"Scripted title"}},
        {"reply":{"text":"held model reply","delayMs":30000}}
    ]}))
    .unwrap();
    let (rig, keys) = rig_with_model("multiple-keys-busy", &fake.base(), Some(script)).await;
    run(
        &rig.core,
        json!({"op":"addMistralKey","name":"Work key","apiKey":WORK}),
    )
    .await;
    let listed = run(&rig.core, json!({"op":"listMistralKeys"})).await;
    let id = listed["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["label"] == "Work key")
        .unwrap()["itemId"]
        .clone();
    let start: Command =
        serde_json::from_value(json!({"type":"call","method":"session/start","params":{}}))
            .unwrap();
    let reply = serde_json::to_value(rig.core.execute(Origin::Ui, start).await.unwrap()).unwrap();
    let session = reply
        .pointer("/result/state/session/id")
        .unwrap()
        .as_str()
        .unwrap();
    let turn: Command = serde_json::from_value(json!({"type":"call","method":"turn/start","params":{"sessionId":session,"message":[{"type":"text","text":"hold"}]}})).unwrap();
    rig.core.execute(Origin::Ui, turn).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = rig
                .core
                .execute(
                    Origin::Ui,
                    Command::ReadSession {
                        session_id: session.to_owned(),
                    },
                )
                .await
                .unwrap();
            if let vw_core_api::CommandResult::Session { snapshot, .. } = state
                && snapshot.state["session"]["status"]["type"] == "running"
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the real chat is working before testing the key switch");
    let error = rig
        .core
        .execute(Origin::Ui, cmd(json!({"op":"useMistralKey","itemId":id})))
        .await
        .unwrap_err();
    assert_eq!(error.server_code.as_deref(), Some("chatsWorking"));
    assert_eq!(
        error.message,
        "Wait until the chats finish, then switch keys."
    );
    assert!(rig.confirmer.shown().is_empty());
    assert_eq!(
        run(&rig.core, json!({"op":"listMistralKeys"})).await,
        listed
    );
    rig.core.shutdown().await;
    keys.shutdown().await;
}

#[tokio::test]
async fn a_new_saved_key_does_not_inherit_rejection_and_next_request_uses_it() {
    let fake = common::fake_mistral::start().await;
    let (rig, keys) = rig("multiple-keys-rejection", &fake.base()).await;
    keys.store_key(SecretString::new("synthetic-bad-key-personal"))
        .await
        .unwrap();
    assert!(keys.test_key().await.is_err());
    assert!(keys.key_was_rejected());
    run(
        &rig.core,
        json!({"op":"addMistralKey","name":"Work key","apiKey":WORK}),
    )
    .await;
    assert!(
        keys.key_was_rejected(),
        "adding does not replace the active key"
    );
    let list = run(&rig.core, json!({"op":"listMistralKeys"})).await;
    let rows = list["keys"].as_array().unwrap();
    assert_eq!(
        rows.iter().find(|k| k["inUse"] == true).unwrap()["lastTest"]["code"],
        401
    );
    let id = rows.iter().find(|k| k["label"] == "Work key").unwrap()["itemId"].clone();
    run(&rig.core, json!({"op":"useMistralKey","itemId":id})).await;
    assert!(!keys.key_was_rejected());
    rig.confirmer.then(Confirmation::Declined);
    let error = rig
        .core
        .execute(
            Origin::Ui,
            cmd(json!({"op":"setupStoreKey","apiKey":PERSONAL})),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ConfirmationDeclined);
    keys.test_key().await.unwrap();
    assert_eq!(
        fake.requests().last().unwrap().authorization.as_deref(),
        Some(format!("Bearer {WORK}").as_str())
    );
    rig.core.shutdown().await;
    keys.shutdown().await;
}

#[tokio::test]
async fn replacing_a_rejected_additional_key_preserves_the_personal_item() {
    let fake = common::fake_mistral::start().await;
    let (rig, keys) = rig("multiple-keys-replace", &fake.base()).await;
    let first = run(&rig.core, json!({"op":"listMistralKeys"})).await["keys"][0]["itemId"].clone();
    run(
        &rig.core,
        json!({"op":"addMistralKey","name":"Work key","apiKey":"synthetic-bad-key-work"}),
    )
    .await;
    let list = run(&rig.core, json!({"op":"listMistralKeys"})).await;
    let id = list["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["label"] == "Work key")
        .unwrap()["itemId"]
        .clone();
    run(&rig.core, json!({"op":"useMistralKey","itemId":id})).await;
    assert!(keys.test_key().await.is_err());
    let confirmations = rig.confirmer.shown().len();
    run(&rig.core, json!({"op":"setupStoreKey","apiKey":WORK})).await;
    assert_eq!(rig.confirmer.shown().len(), confirmations);
    let list = run(&rig.core, json!({"op":"listMistralKeys"})).await;
    assert_eq!(list["keys"].as_array().unwrap().len(), 2);
    assert_eq!(
        list["keys"]
            .as_array()
            .unwrap()
            .iter()
            .find(|k| k["inUse"] == true)
            .unwrap()["itemId"],
        id
    );
    run(&rig.core, json!({"op":"useMistralKey","itemId":first})).await;
    keys.test_key().await.unwrap();
    assert_eq!(
        fake.requests().last().unwrap().authorization.as_deref(),
        Some(format!("Bearer {PERSONAL}").as_str())
    );
    rig.core.shutdown().await;
    keys.shutdown().await;
}
