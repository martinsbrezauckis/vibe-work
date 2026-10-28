//! Read-aloud tool contract through the session credential wire over real Vibe.
//! Synthetic sessions only; absence of the tool must fail before any sound can be requested.
mod agent_tools;
mod common;
mod procmon;

use agent_tools::{Dirs, call, credential_of, rig};
use serde_json::json;

#[tokio::test]
async fn read_aloud_is_announced_and_reports_nobody_listening() {
    let list = vw_agent_tools::mcp::tool_list();
    let spec = list
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "read_aloud")
        .expect("read_aloud must be discoverable");
    assert!(
        spec["description"]
            .as_str()
            .unwrap()
            .contains("read it to me")
    );
    assert_eq!(spec["inputSchema"]["properties"]["text"]["type"], "string");
    let dirs = Dirs::new("read-aloud-wire");
    let rig = rig(&dirs, "wire").await;
    let sid = rig.start_session(&dirs.root, "ask").await;
    let who = credential_of(&rig.injected(0));
    user_turn(&rig.core, &sid).await;
    let result = call(&who, "read_aloud", json!({"text": "Hello."}))
        .await
        .unwrap();
    assert_eq!(result["status"], "nobodyListening");
    assert_eq!(
        result["message"],
        "Reading aloud needs an open Vibe Work window."
    );
    let refused = call(
        &who,
        "read_aloud",
        json!({"sessionId": "another", "text": "Hello."}),
    )
    .await;
    assert!(refused.unwrap_err().contains("this session"));
    assert!(!sid.is_empty());
    let (_api, client) = common::core::debug_api(&rig.core, &dirs.root).await;
    let debug = client.command(json!({"type":"speech", "request":{"op":"readAloud", "sessionId":sid, "text":"Hello."}})).await;
    assert_eq!(debug["result"]["kind"], "reading");
    assert_eq!(debug["result"]["status"], "nobodyListening");
    assert!(rig.confirmer.shown().is_empty());
    rig.core.shutdown().await;
}

#[tokio::test]
async fn key_missing_leaves_the_window_quiet() {
    let dirs = Dirs::new("read-aloud-no-key");
    let rig = agent_tools::rig_with(&dirs, "no-key", |config| {
        config.read_aloud_available = Some(std::sync::Arc::new(|| true));
    })
    .await;
    let sid = rig.start_session(&dirs.root, "ask").await;
    let who = credential_of(&rig.injected(0));
    user_turn(&rig.core, &sid).await;
    let result = call(&who, "read_aloud", json!({"text":"Hello."}))
        .await
        .unwrap();
    assert_eq!(result["status"], "keyMissing");
    assert_eq!(
        result["message"],
        "Reading aloud needs your Mistral key; set it in Settings > Account."
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn explicit_text_is_capped_and_other_sessions_are_refused_without_writing_settings() {
    use common::{fake_mistral, vault::key_service_config};
    use std::sync::Arc;
    use vw_keys::{KeyService, existing::MemoryVibeKeyring};
    use vw_vault::SecretString;
    let dirs = Dirs::new("read-aloud-text");
    let fake = fake_mistral::start().await;
    let service = KeyService::start(key_service_config(
        &dirs.root,
        &fake.base(),
        Arc::new(MemoryVibeKeyring::default()),
    ))
    .await
    .unwrap();
    service
        .create_vault(SecretString::new("synthetic password"))
        .await
        .unwrap();
    let settings = dirs.root.join("settings.json");
    std::fs::write(&settings, "{\"narration\":false}").unwrap();
    let rig = agent_tools::rig_with(&dirs, "text", |config| {
        config.keys = Some(service.clone());
        config.read_aloud_available = Some(Arc::new(|| true));
    })
    .await;
    let sid = rig.start_session(&dirs.root, "ask").await;
    let who = credential_of(&rig.injected(0));
    user_turn(&rig.core, &sid).await;
    let mut key_events = service.subscribe();
    let missing = call(&who, "read_aloud", json!({"text":"Hello."}))
        .await
        .unwrap();
    assert_eq!(missing["status"], "keyMissing");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), key_events.recv())
            .await
            .is_err(),
        "a missing key must not open setup or emit key-needed"
    );
    service
        .store_key(SecretString::new("synthetic-read-aloud-key"))
        .await
        .unwrap();
    let busy = reading_window(&rig.core, false, true);
    let refused = call(&who, "read_aloud", json!({"text":"Hello."}))
        .await
        .unwrap();
    assert_eq!(refused["status"], "voiceUnavailable");
    assert_eq!(refused["message"], "Another reading is still playing.");
    busy.await.unwrap();
    let unavailable = listening_window(&rig.core, false);
    let refusal = call(&who, "read_aloud", json!({"text":"Hello."}))
        .await
        .unwrap();
    assert_eq!(refusal["status"], "voiceUnavailable");
    assert_eq!(
        refusal["message"],
        "Reading aloud is unavailable; try again in a minute."
    );
    unavailable.await.unwrap();
    let window = listening_window(&rig.core, true);
    let result = call(&who, "read_aloud", json!({"text":"🙂".repeat(10_001)}))
        .await
        .unwrap();
    assert_eq!(result["status"], "started");
    let vw_core_api::CoreEvent::ReadAloud {
        session_id,
        text,
        truncated,
        ..
    } = window.await.unwrap()
    else {
        panic!("reading event")
    };
    let event = (session_id, text, truncated);
    assert_eq!(event.0, sid);
    assert_eq!(event.1.chars().count(), 10_000);
    assert!(event.2);
    assert!(
        call(&who, "read_aloud", json!({"sessionId":"other"}))
            .await
            .unwrap_err()
            .contains("this session")
    );
    assert_eq!(
        std::fs::read_to_string(settings).unwrap(),
        "{\"narration\":false}"
    );
    assert!(rig.confirmer.shown().is_empty());
    rig.core.shutdown().await;
}

#[tokio::test]
async fn replay_read_it_to_me_calls_the_tool_for_the_previous_reply() {
    use common::{fake_mistral, vault::key_service_config};
    use std::sync::Arc;
    use vw_core_api::{Command, CoreEvent, Origin};
    use vw_keys::{KeyService, existing::MemoryVibeKeyring};
    use vw_vault::SecretString;
    let dirs = Dirs::new("read-aloud-replay");
    let fake = fake_mistral::start().await;
    let keys = KeyService::start(key_service_config(
        &dirs.root,
        &fake.base(),
        Arc::new(MemoryVibeKeyring::default()),
    ))
    .await
    .unwrap();
    keys.create_vault(SecretString::new("synthetic password"))
        .await
        .unwrap();
    keys.store_key(SecretString::new("synthetic-read-aloud-key"))
        .await
        .unwrap();
    let rig = agent_tools::rig_with(&dirs, "replay", |config| {
        config.keys = Some(keys);
        config.read_aloud_available = Some(Arc::new(|| true));
    })
    .await;
    let sid = rig.start_session(&dirs.root, "auto-approve").await;
    let mut events = rig.core.subscribe(None);
    for prompt in ["Explain the result.", "read it to me"] {
        rig.core.execute(Origin::Ui, serde_json::from_value::<Command>(json!({"type":"startTurn", "sessionId":sid, "message":[{"type":"text", "text":prompt}]})).unwrap()).await.unwrap();
        let mut read = None;
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                match events.next().await {
                    Some(CoreEvent::CallbackOpened { .. }) => panic!("reading must not ask for approval"),
                    Some(CoreEvent::ReadAloud {
                        request_id, text, session_id, ..
                    }) => {
                        rig.core.execute(Origin::Ui, serde_json::from_value::<Command>(json!({"type":"speech", "request":{"op":"readingReady", "requestId":request_id, "available":true}})).unwrap()).await.unwrap();
                        read = Some((text, session_id));
                    },
                    Some(CoreEvent::Change {
                        change: vw_core_api::SessionChange::TurnUpserted { turn, .. },
                    }) if turn["status"] == "completed" => break,
                    _ => {}
                }
            }
        })
        .await
        .unwrap_or_else(|error| {
            panic!("replay {prompt:?}: {error}; {:?}", rig.core.status().server)
        });
        if prompt == "Explain the result." {
            rig.core
                .execute(
                    Origin::Ui,
                    serde_json::from_value::<Command>(
                        json!({"type":"setAccessMode", "sessionId":sid, "agentName":"ask"}),
                    )
                    .unwrap(),
                )
                .await
                .unwrap();
        }
        if prompt == "read it to me" {
            if read.is_none() {
                read = tokio::time::timeout(std::time::Duration::from_millis(500), async {
                    loop {
                        if let Some(CoreEvent::ReadAloud {
                            text, session_id, ..
                        }) = events.next().await
                        {
                            break Some((text, session_id));
                        }
                    }
                })
                .await
                .ok()
                .flatten();
            }
            assert_eq!(
                read,
                Some(("approved: approve".to_owned(), sid.clone())),
                "the replay must call read_aloud, reading the prior answer instead of its acknowledgement"
            );
        }
    }
    rig.core.shutdown().await;
}

/// A synthetic window answers only the first-speech receipt, never a native dialog.
fn listening_window(
    core: &vw_core_api::Core,
    available: bool,
) -> tokio::task::JoinHandle<vw_core_api::CoreEvent> {
    reading_window(core, available, false)
}

fn reading_window(
    core: &vw_core_api::Core,
    available: bool,
    busy: bool,
) -> tokio::task::JoinHandle<vw_core_api::CoreEvent> {
    let core = core.clone();
    let mut events = core.subscribe(None);
    tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(event @ vw_core_api::CoreEvent::ReadAloud { .. }) = events.next().await {
                    let vw_core_api::CoreEvent::ReadAloud { ref request_id, .. } = event else { unreachable!() };
                    core.execute(vw_core_api::Origin::Ui, serde_json::from_value::<vw_core_api::Command>(json!({"type":"speech", "request":{"op":"readingReady", "requestId":request_id, "available":available, "busy":busy}})).unwrap()).await.unwrap();
                    return event;
                }
            }
        }).await.unwrap()
    })
}

/// Completes a synthetic real user turn, answering the fake's ordinary edit card if present.
async fn user_turn(core: &vw_core_api::Core, session: &str) {
    use vw_core_api::{CoreEvent, SessionChange};
    let mut events = core.subscribe(None);
    execute(
        core,
        json!({"type":"startTurn", "sessionId":session,
        "message":[{"type":"text", "text":"Please explain the result."}]}),
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            match events.next().await {
                Some(CoreEvent::CallbackOpened { session_id, callback_id, .. }) => {
                    execute(core, json!({"type":"answerCallback", "sessionId":session_id,
                        "callbackId":callback_id, "output":{"type":"approval", "decision":{"type":"deny"}}})).await;
                }
                Some(CoreEvent::Change { change: SessionChange::TurnUpserted { turn, .. } })
                    if turn["status"] == "completed" => break,
                _ => {}
            }
        }
    }).await.unwrap();
}

async fn execute(
    core: &vw_core_api::Core,
    command: serde_json::Value,
) -> vw_core_api::CommandResult {
    core.execute(
        vw_core_api::Origin::Ui,
        serde_json::from_value(command).unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn concurrent_and_sequential_tool_readings_share_one_real_user_turn_budget() {
    use common::{fake_mistral, vault::key_service_config};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use vw_core_api::CoreEvent;
    use vw_keys::{KeyService, existing::MemoryVibeKeyring};
    use vw_vault::SecretString;
    let dirs = Dirs::new("read-aloud-budget");
    let fake = fake_mistral::start().await;
    let keys = KeyService::start(key_service_config(
        &dirs.root,
        &fake.base(),
        Arc::new(MemoryVibeKeyring::default()),
    ))
    .await
    .unwrap();
    keys.create_vault(SecretString::new("synthetic password"))
        .await
        .unwrap();
    keys.store_key(SecretString::new("synthetic-read-aloud-key"))
        .await
        .unwrap();
    let rig = agent_tools::rig_with(&dirs, "budget", |config| {
        config.keys = Some(keys);
        config.read_aloud_available = Some(Arc::new(|| true));
    })
    .await;
    let sid = rig.start_session(&dirs.root, "ask").await;
    let who = credential_of(&rig.injected(0));
    user_turn(&rig.core, &sid).await;
    user_turn(&rig.core, &sid).await;
    user_turn(&rig.core, &sid).await;
    let count = Arc::new(AtomicUsize::new(0));
    let mut events = rig.core.subscribe(None);
    let window = {
        let core = rig.core.clone();
        let count = count.clone();
        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                if let CoreEvent::ReadAloud {
                    request_id, text, ..
                } = event
                {
                    count.fetch_add(1, Ordering::SeqCst);
                    execute(&core, json!({"type":"speech", "request":{"op":"speak", "text":text, "voice":null}})).await;
                    execute(&core, json!({"type":"speech", "request":{"op":"readingReady", "requestId":request_id, "available":true}})).await;
                }
            }
        })
    };
    let (one, two) = tokio::join!(
        call(&who, "read_aloud", json!({})),
        call(&who, "read_aloud", json!({"text":"An explicit part."})),
    );
    let results = [one.unwrap(), two.unwrap()];
    assert_eq!(
        results.iter().filter(|r| r["status"] == "started").count(),
        1,
        "one real user turn can start only one reading"
    );
    let repeat = "This reply was already read aloud; ask again in a new message to hear it again.";
    assert_eq!(
        results.iter().find(|r| r["status"] != "started").unwrap()["message"],
        repeat
    );
    for args in [json!({}), json!({"text":"Different explicit text."})] {
        let refused = call(&who, "read_aloud", args).await.unwrap();
        assert_eq!(refused["message"], repeat);
    }
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "refused calls emit no event"
    );
    assert_eq!(
        fake.speech.tts_bodies.lock().unwrap().len(),
        1,
        "refused calls make no TTS request"
    );
    user_turn(&rig.core, &sid).await;
    assert_eq!(
        call(&who, "read_aloud", json!({})).await.unwrap()["status"],
        "started"
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(fake.speech.tts_bodies.lock().unwrap().len(), 2);
    user_turn(&rig.core, &sid).await;
    assert_eq!(
        call(
            &who,
            "read_aloud",
            json!({"text":"An explicit first reading."})
        )
        .await
        .unwrap()["status"],
        "started"
    );
    assert_eq!(
        call(&who, "read_aloud", json!({})).await.unwrap()["status"],
        "alreadyRead"
    );
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert_eq!(fake.speech.tts_bodies.lock().unwrap().len(), 3);
    window.abort();
    rig.core.shutdown().await;
}

#[tokio::test]
async fn debug_owner_can_repeat_in_one_turn_and_receives_the_window_busy_refusal() {
    use common::{fake_mistral, vault::key_service_config};
    use std::sync::Arc;
    use vw_keys::{KeyService, existing::MemoryVibeKeyring};
    use vw_vault::SecretString;
    let dirs = Dirs::new("read-aloud-owner");
    let fake = fake_mistral::start().await;
    let keys = KeyService::start(key_service_config(
        &dirs.root,
        &fake.base(),
        Arc::new(MemoryVibeKeyring::default()),
    ))
    .await
    .unwrap();
    keys.create_vault(SecretString::new("synthetic password"))
        .await
        .unwrap();
    keys.store_key(SecretString::new("synthetic-read-aloud-key"))
        .await
        .unwrap();
    let rig = agent_tools::rig_with(&dirs, "owner", |config| {
        config.keys = Some(keys);
        config.read_aloud_available = Some(Arc::new(|| true));
    })
    .await;
    let sid = rig.start_session(&dirs.root, "ask").await;
    let who = credential_of(&rig.injected(0));
    user_turn(&rig.core, &sid).await;
    let first = listening_window(&rig.core, true);
    assert_eq!(
        call(&who, "read_aloud", json!({"text":"Agent reading."}))
            .await
            .unwrap()["status"],
        "started"
    );
    first.await.unwrap();
    let (_api, client) = common::core::debug_api(&rig.core, &dirs.root).await;
    for text in [serde_json::Value::Null, json!("Owner's explicit text.")] {
        let window = listening_window(&rig.core, true);
        let result = client.command(json!({"type":"speech", "request":{"op":"readAloud", "sessionId":sid, "text":text}})).await;
        assert_eq!(
            result["result"]["status"], "started",
            "the owner bypasses the spent agent turn budget"
        );
        window.await.unwrap();
    }
    let busy = reading_window(&rig.core, false, true);
    let refused = client.command(json!({"type":"speech", "request":{"op":"readAloud", "sessionId":sid, "text":"One too many."}})).await;
    assert_eq!(refused["result"]["status"], "voiceUnavailable");
    assert_eq!(
        refused["result"]["message"],
        "Another reading is still playing."
    );
    busy.await.unwrap();
    assert_eq!(
        call(&who, "read_aloud", json!({"text":"Agent retry."}))
            .await
            .unwrap()["status"],
        "alreadyRead"
    );
    assert!(rig.confirmer.shown().is_empty());
    rig.core.shutdown().await;
    let manual = include_str!("../../../../docs/public/DEBUG_API.md");
    assert!(
        manual.contains("The owner can request another reading in the same user turn"),
        "document the owner's turn-budget exception"
    );
    assert!(
        manual.contains("At most 2 requested readings can be outstanding across all sessions"),
        "document the shared window limit"
    );
}
