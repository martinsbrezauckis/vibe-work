//! Core/gate regressions without Vibe: the pending-install core has no server to confirm answers.
//! Exercises the real card and automatic adapters, registry, transport failure and grants; called by cargo test.
use super::*;
use crate::{Core, CoreConfig, Origin};
use vw_ask::AnsweredBy;
use vw_gate::{CallbackFault, Confirmation, ConfirmationRequest, Confirmer, GateError};
use vw_supervisor::{Discovery, NoSecret, SupervisorConfig};
use vw_vibe_install::{InstallerConfig, Limits, Sources, VibeInstaller};

struct Decline;
impl Confirmer for Decline {
    fn confirm(
        &self,
        _: &ConfirmationRequest,
    ) -> impl std::future::Future<Output = Confirmation> + Send {
        std::future::ready(Confirmation::Declined)
    }
}

async fn core() -> (Core, std::path::PathBuf) {
    // A counter as well as the clock: macOS's clock ticks in microseconds, and these tests run in parallel.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "vw-dbk4-delivery-{}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let installer = VibeInstaller::new(InstallerConfig {
        root: root.join("install"),
        sources: Sources::pinned(&vw_vibe_install::this_target()),
        parent_env: Vec::new(),
        extra_env: Vec::new(),
        discovery: Discovery {
            explicit: None,
            path_var: None,
            home: None,
        },
        limits: Limits::default(),
    });
    assert!(installer.server().is_none());
    let config = CoreConfig {
        vibe_install: Some(installer),
        connection_wait: Duration::from_millis(5),
        ..Default::default()
    };
    let supervisor =
        SupervisorConfig::new(root.join("absent-server"), root.join("vibe"), root.clone());
    (
        Core::start(supervisor, Arc::new(NoSecret), Decline, config)
            .await
            .unwrap(),
        root,
    )
}

fn open(shared: &Arc<Shared>, id: &str) -> Value {
    let entry = json!({
        "id": id, "callbackId": id, "sessionId": "session", "type": "callback",
        "detail": {"kind": "approval", "effect": {"toolName": "file_system.write_file"},
            "requiredPermissions": [], "choices": ["approve", "approve_for_session", "deny"]},
        "state": {"status": "open"}
    });
    shared
        .gate
        .register_callback(&json!({"callback": entry}))
        .unwrap();
    crate::ask::callback_opened(shared, &entry, "approval", "session");
    entry
}

fn unconfirmed(shared: &Shared, id: &str) {
    assert!(shared.asks.get(&format!("callback:{id}")).is_none());
    let record = shared.asks.outcome(&format!("callback:{id}")).unwrap();
    assert_eq!(
        serde_json::to_value(record.outcome).unwrap()["type"],
        "answerNotConfirmed"
    );
}

#[tokio::test]
async fn a_reserved_answer_has_no_decision_and_cannot_be_clicked_or_closed_again() {
    let (core, root) = core().await;
    let shared = core.shared.clone();
    open(&shared, "in-flight");
    let delivery = shared
        .asks
        .reserve("callback:in-flight", AnsweredBy::Person)
        .unwrap();
    assert!(
        shared.asks.records().is_empty(),
        "no final decision before callback/result succeeds"
    );
    assert!(
        crate::ask::cards::answer(
            &shared,
            Origin::Ui,
            "session".into(),
            "in-flight".into(),
            crate::one_ask::approve_once()
        )
        .await
        .is_err()
    );
    crate::ask::callback_closed(&shared, "in-flight");
    assert!(
        shared.asks.records().is_empty(),
        "a close cannot override delivery in flight"
    );
    assert_eq!(
        shared.gate.open_callbacks().len(),
        1,
        "a second click cannot claim the reserved callback"
    );
    drop(delivery);
    unconfirmed(&shared, "in-flight");
    drop(shared);
    drop(core);
    std::fs::remove_dir_all(root).unwrap();
}

async fn failed_decision(decision: &str) {
    let (core, root) = core().await;
    let shared = &core.shared;
    open(shared, decision);
    let output = json!({"type":"approval", "decision":{"type":decision}});
    assert!(
        crate::ask::cards::answer(
            shared,
            Origin::Ui,
            "session".into(),
            decision.into(),
            output
        )
        .await
        .is_err()
    );
    unconfirmed(shared, decision);
    assert!(
        shared.ask_grants.list().is_empty(),
        "failed delivery must keep no grant"
    );
    assert!(
        shared.ask_grants.offers().is_empty(),
        "failed delivery must leave no grant offer"
    );
    assert!(matches!(
        shared
            .gate
            .answer_callback("session", decision, crate::one_ask::approve_once())
            .await,
        Err(GateError::Callback(
            CallbackFault::AlreadyResolved { .. } | CallbackFault::UnknownCallback { .. }
        ))
    ));
    drop(core);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn failed_yes_is_not_a_final_decision() {
    failed_decision("approve").await;
}
#[tokio::test]
async fn failed_no_is_not_a_final_decision() {
    failed_decision("deny").await;
}
#[tokio::test]
async fn failed_session_answer_keeps_no_grant() {
    failed_decision("approve_for_session").await;
}

#[tokio::test]
async fn failed_automatic_answer_never_reopens_a_claimed_card_or_counts_a_grant_use() {
    let (core, root) = core().await;
    let shared = core.shared.clone();
    open(&shared, "grant-source");
    let pending = shared.ask_grants.claim("grant-source").unwrap();
    let grant = shared
        .ask_grants
        .keep(pending, AnsweredBy::Person, shared.asks.now_ms())
        .unwrap();
    shared.asks.settle(
        "callback:grant-source",
        crate::ask::allowed(AnsweredBy::Person, vw_ask::Scope::Session),
    );
    let entry = open(&shared, "automatic");
    assert!(matches!(
        carried_for(&shared, "session", &entry, "approval"),
        Some(Carried::Grant(_))
    ));
    let mut events = shared.events.subscribe();
    carry_answer(
        shared.clone(),
        "session".into(),
        "automatic".into(),
        Carried::Grant(grant.id),
    )
    .await;
    let mut reopened = false;
    while let Ok(event) = events.try_recv() {
        reopened |= matches!(event, CoreEvent::CallbackOpened { .. });
    }
    assert!(!reopened, "a claimed callback must never be offered again");
    unconfirmed(&shared, "automatic");
    assert_eq!(shared.ask_grants.list()[0].uses, 0);
    assert_eq!(shared.ask_grants.list()[0].last_used_at, None);
    assert!(matches!(
        shared
            .gate
            .answer_callback("session", "automatic", crate::one_ask::approve_once())
            .await,
        Err(GateError::Callback(CallbackFault::AlreadyResolved { .. }))
    ));
    drop(shared);
    drop(core);
    std::fs::remove_dir_all(root).unwrap();
}
