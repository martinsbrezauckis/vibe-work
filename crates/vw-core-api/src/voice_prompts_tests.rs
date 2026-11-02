//! Core ownership and prompt lifetime regression tests, without a microphone or Vibe process.
use crate::{Command, Origin};
use serde_json::{Value, json};

async fn run(
    core: &crate::Core,
    origin: Origin,
    request: Value,
) -> Result<Value, crate::CoreError> {
    let Command::Speech { request } =
        serde_json::from_value::<Command>(json!({"type":"speech","request":request})).unwrap()
    else {
        panic!("speech command");
    };
    core.speech(origin, request)
        .await
        .map(|r| serde_json::to_value(r).unwrap())
}
async fn rig() -> crate::Core {
    rig_with_clock(std::sync::Arc::new(crate::SystemAskClock::default())).await
}
async fn rig_with_clock(clock: std::sync::Arc<dyn crate::AskClock>) -> crate::Core {
    let core = crate::offline_core::start_with("voice-prompt-policy", |config, _| {
        config.ask_clock = clock;
    })
    .await;
    let entry = json!({"id":"c1","sessionId":"s","callbackId":"c1","type":"callback","createdAt":1000,"title":"Read a file",
        "detail":{"kind":"approval","effect":{"toolName":"file_system.read_file","input":{"path":"hello.txt"}},"choices":["approve","deny"]},"state":{"status":"open"}});
    let _ = core.shared.store.load(json!({"session":{"id":"s","harness":"unified","createdAt":1000,"bumpedAt":2000,"status":{"type":"idle"}},"eventId":3,"turns":[],"history":[entry]}),vw_session::LoadKind::New).unwrap();
    crate::ask::cards::callback_opened(&core.shared, &entry, "approval", "s");
    core
}

/// The ask clock produces epoch timestamps and follows wall time. A challenge lifetime must not use it.
#[tokio::test(start_paused = true)]
async fn challenge_lifetime_ignores_wall_clock_steps() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;
    static WALL: AtomicU64 = AtomicU64::new(1_800_000_000_000);
    fn wall() -> u64 {
        WALL.load(Ordering::SeqCst)
    }
    let mut lifetimes = Vec::new();
    for forward in [true, false] {
        WALL.store(1_800_000_000_000, Ordering::SeqCst);
        let clock = Arc::new(crate::SystemAskClock::with_wall(wall));
        let core = rig_with_clock(clock).await;
        // A timestamp clock may have followed a forward correction before the challenge was prepared.
        if !forward {
            WALL.fetch_add(3_600_000, Ordering::SeqCst);
        }
        run(
            &core,
            Origin::Ui,
            json!({"op":"promptSession","sessionId":"s"}),
        )
        .await
        .unwrap();
        let prepared = run(
            &core,
            Origin::Ui,
            json!({"op":"promptPrepare","sessionId":"s","callbackId":"c1"}),
        )
        .await
        .unwrap();
        let ticket = prepared["result"]["ticket"].as_str().unwrap();
        if forward {
            WALL.fetch_add(3_600_000, Ordering::SeqCst);
        } else {
            WALL.fetch_sub(7_200_000, Ordering::SeqCst);
        }
        tokio::time::advance(Duration::from_secs(1)).await;
        let fresh = super::checked(&core.shared, ticket).is_ok();
        tokio::time::advance(Duration::from_secs(120)).await;
        let expired = super::checked(&core.shared, ticket).is_err();
        lifetimes.push((fresh, expired));
        core.shutdown().await;
    }
    assert_eq!(
        lifetimes,
        [(true, true), (true, true)],
        "forward and backward steps must neither shorten nor extend a challenge"
    );
}
#[tokio::test]
async fn fresh_numbers_require_the_owned_voice_chat_and_never_allow_debug_control() {
    let core = rig().await;
    let prepare = json!({"op":"promptPrepare","sessionId":"s","callbackId":"c1"});
    assert!(run(&core, Origin::Ui, prepare.clone()).await.is_err());
    assert!(
        run(
            &core,
            Origin::DebugApi,
            json!({"op":"promptSession","sessionId":"s"})
        )
        .await
        .is_err()
    );
    run(
        &core,
        Origin::Ui,
        json!({"op":"promptSession","sessionId":"s"}),
    )
    .await
    .unwrap();
    let first = run(&core, Origin::Ui, prepare.clone()).await.unwrap();
    assert!(first["result"]["ticket"].is_string());
    let number = first["result"]["number"].as_u64().unwrap();
    assert!((10..=99).contains(&number));
    run(
        &core,
        Origin::Ui,
        json!({"op":"promptCancel","ticket":first["result"]["ticket"]}),
    )
    .await
    .unwrap();
    let next = run(&core, Origin::Ui, prepare).await.unwrap();
    assert_ne!(next["result"]["number"], number);
    assert!(
        run(
            &core,
            Origin::Ui,
            json!({"op":"promptPrepare","sessionId":"other","callbackId":"c1"})
        )
        .await
        .is_err()
    );
    run(
        &core,
        Origin::Ui,
        json!({"op":"promptSession","sessionId":null}),
    )
    .await
    .unwrap();
    assert!(run(&core,Origin::Ui,json!({"op":"promptStart","ticket":next["result"]["ticket"],"sampleRate":16000,"language":null})).await.is_err());
    assert_eq!(core.ask_records(), []);
    core.shutdown().await;
}

#[tokio::test]
async fn policy_blocks_native_secrets_network_questions_and_unclassified_approval() {
    let core = rig().await;
    let mut ask = core.asks()[0].clone();
    let detail = json!({"kind":"approval","effect":{"toolName":"file_system.read_file"}});
    assert_eq!(super::policy(&ask, &detail), Some(true));
    for tool in [
        "mcp_unknown.send",
        "browser.open",
        "network.upload",
        "secret.run",
    ] {
        assert_eq!(
            super::policy(&ask, &json!({"kind":"approval","effect":{"toolName":tool}})),
            None
        );
    }
    for tool in ["file_system.bash", "a_future_tool"] {
        assert_eq!(
            super::policy(&ask, &json!({"kind":"approval","effect":{"toolName":tool}})),
            Some(false)
        );
    }
    assert_eq!(
        super::policy(
            &ask,
            &json!({"kind":"approval","effect":{"toolName":"file_system.read_file"},"reason":"above the access level"})
        ),
        None
    );
    ask.surface = vw_ask::Surface::Native;
    assert_eq!(super::policy(&ask, &detail), None);
    ask.surface = vw_ask::Surface::Card;
    ask.person_only = true;
    assert_eq!(super::policy(&ask, &detail), None);
    ask.person_only = false;
    ask.kind = "user_input".into();
    assert_eq!(super::policy(&ask, &detail), None);
    core.shutdown().await;
}

#[tokio::test]
async fn a_closed_card_expired_challenge_or_foreign_stream_cannot_answer() {
    let core = rig().await;
    run(
        &core,
        Origin::Ui,
        json!({"op":"promptSession","sessionId":"s"}),
    )
    .await
    .unwrap();
    let prepared = run(
        &core,
        Origin::Ui,
        json!({"op":"promptPrepare","sessionId":"s","callbackId":"c1"}),
    )
    .await
    .unwrap();
    let ticket = prepared["result"]["ticket"].as_str().unwrap();
    assert!(
        run(
            &core,
            Origin::Ui,
            json!({"op":"promptAnswer","ticket":ticket,"streamId":"unrelated-dictation"})
        )
        .await
        .is_err()
    );
    core.shared
        .voice_prompts
        .lock()
        .current
        .as_mut()
        .unwrap()
        .until = tokio::time::Instant::now();
    assert!(super::checked(&core.shared, ticket).is_err());
    core.shared
        .voice_prompts
        .lock()
        .current
        .as_mut()
        .unwrap()
        .until = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    core.shared.asks.settle(
        "callback:c1",
        vw_ask::Outcome::Cancelled {
            reason: "closed".into(),
        },
    );
    assert!(super::checked(&core.shared, ticket).is_err());
    assert!(
        core.ask_records()
            .iter()
            .all(|r| !matches!(r.outcome, vw_ask::Outcome::Allowed { .. }))
    );
    core.shutdown().await;
}

#[test]
fn challenge_numbers_are_uniform_and_never_repeat_the_previous() {
    let mut counts = [0; 100];
    for byte in 0..=255 {
        if let Some(number) = super::number_candidate(byte, 47) {
            counts[usize::from(number)] += 1;
        }
    }
    for (number, count) in counts.iter().enumerate().skip(10) {
        assert_eq!(*count, if number == 47 { 0 } else { 2 }, "number {number}");
    }
}
