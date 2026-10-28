//! Callback admission policy without a Vibe process. Tests the actual adapter, registry behavior.
//! Connector authorization uses notifications in pinned Vibe; its obsolete callback contract remains refused.
use serde_json::json;
use std::sync::Arc;
use vw_ask::Choice;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

struct NoNativePrompt;
impl Confirmer for NoNativePrompt {
    fn confirm(
        &self,
        _: &ConfirmationRequest,
    ) -> impl std::future::Future<Output = Confirmation> + Send {
        std::future::poll_fn(|_| -> std::task::Poll<Confirmation> {
            panic!("callback policy must not open a native prompt")
        })
    }
}

#[tokio::test(start_paused = true)]
async fn foreign_callback_kinds_never_open_an_ask_and_unscoped_bash_stays_once_only() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/tmp/callback-policy-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let config = crate::CoreConfig {
        start_vibe: false,
        ..crate::CoreConfig::default()
    };
    let core = crate::Core::start(
        vw_supervisor::SupervisorConfig::new(
            dir.join("no-server-requested"),
            dir.join("vibe-home"),
            dir.clone(),
        ),
        Arc::new(vw_supervisor::NoSecret),
        NoNativePrompt,
        config,
    )
    .await
    .unwrap();
    assert!(core.status().server.pid.is_none());
    for kind in ["connector_auth", "a_future_callback"] {
        super::cards::callback_opened(
            &core.shared,
            &json!({"id":kind,"sessionId":"session","detail":{"kind":kind}}),
            kind,
            "session",
        );
    }
    tokio::time::advance(std::time::Duration::from_secs(601)).await;
    tokio::task::yield_now().await;
    assert!(
        core.asks().is_empty(),
        "foreign callbacks opened asks: {:?}",
        core.asks()
    );
    assert!(
        core.ask_records().is_empty(),
        "unanswerable callbacks leave no closed record"
    );
    super::cards::callback_opened(
        &core.shared,
        &json!({"id":"bash","sessionId":"session","detail":{"kind":"approval","effect":{"toolName":"file_system.bash","input":{"command":"probe"}},"requiredPermissions":[]}}),
        "approval",
        "session",
    );
    assert_eq!(core.asks()[0].offers, vec![Choice::Once, Choice::No]);
    assert!(
        core.answer_ask(
            "callback:bash",
            crate::Answer {
                answer: Choice::Session,
                note: None
            }
        )
        .await
        .is_err()
    );
    assert!(core.status().server.pid.is_none());
    core.shutdown().await;
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}
