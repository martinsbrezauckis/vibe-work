//! Every person-only ask kind keeps its answer for the person on every callback route.
//! PREDICT (before the shared check in `cards::answer_by`): the vault save card's own handler refused only a request
//! from `Origin::DebugApi`, so a window click the Debug API drives (`Origin::Ui` while the host's probe says a `ui/*`
//! request is served) reserved and answered the person-only card. Other kinds reached no person check on this route
//! at all, so every row below fails without the shared check.
//! Uses a core without Vibe and asks opened directly in the shared registry; no native prompt is shown.
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vw_ask::{Answer, AnsweredBy, Choice};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

use crate::Origin;

struct NoNativePrompt;
impl Confirmer for NoNativePrompt {
    fn confirm(
        &self,
        _: &ConfirmationRequest,
    ) -> impl std::future::Future<Output = Confirmation> + Send {
        std::future::poll_fn(|_| -> std::task::Poll<Confirmation> {
            panic!("the person-only routes must not open a native prompt")
        })
    }
}

async fn core(tag: &str) -> crate::Core {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/tmp/person-only-{tag}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    Box::pin(crate::Core::start(
        vw_supervisor::SupervisorConfig::new(
            dir.join("no-server-requested"),
            dir.join("vibe-home"),
            dir.clone(),
        ),
        Arc::new(vw_supervisor::NoSecret),
        NoNativePrompt,
        crate::CoreConfig {
            start_vibe: false,
            ..crate::CoreConfig::default()
        },
    ))
    .await
    .unwrap()
}

/// Every person-only policy kind, the window's own card (`vault_save`) first.
fn person_only_kinds() -> Vec<&'static str> {
    let mut kinds: Vec<_> = vw_ask::policies()
        .filter(|kind| vw_ask::policy(kind).is_some_and(|p| p.person_only))
        .collect();
    kinds.sort_by_key(|kind| *kind != "vault_save");
    kinds
}

/// Answers the person-only card `callback` through every route but the person's: a window click the Debug API
/// drives, the Debug API's own answer and a spoken answer, with Allow and with No. Each must be refused.
async fn refused_on_every_route(
    core: &crate::Core,
    driving: &AtomicBool,
    kind: &str,
    callback: &str,
) {
    for decision in ["approve", "deny"] {
        let output = json!({"type":"approval","decision":{"type":decision}});
        for (origin, drive) in [(Origin::Ui, true), (Origin::DebugApi, false)] {
            driving.store(drive, Ordering::SeqCst);
            // The callback route `answerCallback` dispatches to (a core without Vibe refuses the command itself
            // before it).
            let answer = Box::pin(super::cards::answer(
                &core.shared,
                origin,
                "session".into(),
                callback.into(),
                output.clone(),
            ))
            .await;
            let error = answer.expect_err(&format!("{kind} {decision} {origin:?}"));
            assert_eq!(
                error.server_code.as_deref(),
                Some(crate::vault_scope::VAULT_USER_ONLY),
                "{kind} {decision} {origin:?}: {error:?}"
            );
        }
        driving.store(false, Ordering::SeqCst);
        let spoken = Box::pin(super::cards::answer_by(
            &core.shared,
            Origin::Ui,
            "session".into(),
            callback.into(),
            output,
            AnsweredBy::Voice,
        ))
        .await;
        assert!(spoken.is_err(), "{kind} {decision} spoken");
    }
}

#[tokio::test]
async fn no_person_only_card_takes_a_driven_window_click_a_debug_api_or_a_spoken_answer() {
    let kinds = person_only_kinds();
    // The list this test walks: the vault save card, every vault dialog, pairing and a site's local network access.
    for expected in [
        "vault_save",
        "vibe-work/mcpServerKey",
        "vibe-work/captureSecret",
        "vibe-work/replaceVaultSecret",
        "vibe-work/secretRun",
        "chrome/control/pair",
        "vibe-work/localNetworkAccess",
    ] {
        assert!(kinds.contains(&expected), "{expected} is person-only");
    }
    let core = Box::pin(core("kinds")).await;
    let driving = Arc::new(AtomicBool::new(false));
    let probe = driving.clone();
    core.set_window_drive(move || probe.load(Ordering::SeqCst));
    let mut handles = Vec::new();
    for (n, kind) in kinds.iter().enumerate() {
        let callback = format!("person-only-{n}");
        let handle = core.shared.asks.open_own(
            kind,
            Some("session".into()),
            vw_ask::Requester::Agent,
            vec![Choice::Once, Choice::No],
            Some(format!("callback:{callback}")),
        );
        assert!(handle.ask.person_only, "{kind}");
        Box::pin(refused_on_every_route(&core, &driving, kind, &callback)).await;
        assert!(
            Box::pin(core.answer_ask(
                &handle.ask.id,
                Answer {
                    answer: Choice::Once,
                    note: None
                }
            ))
            .await
            .is_err(),
            "{kind} /asks"
        );
        handles.push(handle);
    }
    let open = core.asks();
    for handle in &handles {
        assert!(
            open.iter().any(|ask| ask.id == handle.ask.id),
            "{} stays open for the person",
            handle.ask.kind
        );
    }
    assert!(core.ask_records().is_empty(), "{:?}", core.ask_records());
    // Not vacuous: an ask that is not person-only passes this check (and is refused later, as an unknown callback).
    let ordinary = core.shared.asks.open_own(
        "approval",
        Some("session".into()),
        vw_ask::Requester::Agent,
        vec![Choice::Once, Choice::No],
        Some("callback:ordinary".into()),
    );
    assert!(!ordinary.ask.person_only);
    driving.store(true, Ordering::SeqCst);
    let error = Box::pin(super::cards::answer(
        &core.shared,
        Origin::Ui,
        "session".into(),
        "ordinary".into(),
        json!({"type":"approval","decision":{"type":"approve"}}),
    ))
    .await
    .expect_err("no such Vibe callback");
    assert_ne!(
        error.server_code.as_deref(),
        Some(crate::vault_scope::VAULT_USER_ONLY)
    );
    drop(handles);
    drop(ordinary);
    core.shutdown().await;
}
