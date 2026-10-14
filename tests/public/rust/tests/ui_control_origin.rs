//! UI control capability provenance through a fake Vibe host.
//! PREDICT: a pending request can act, while missing, forged, expired, or answered claims are denied.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::fake::{fake_bin, scratch};
use serde_json::{Value, json};
use vw_core_api::{Command, CommandResult, CoreConfig, ErrorCode};
use vw_host::{Host, HostConfig, UiOutcome};
use vw_supervisor::{NoSecret, SupervisorConfig};

fn shell(sid: &str) -> Command {
    Command::Call {
        method: "session/shellCommand".to_owned(),
        params: json!({"sessionId": sid, "command": "echo x"}),
        confirmation_token: None,
    }
}

fn add() -> Command {
    Command::Call {
        method: "mcp/add".to_owned(),
        params: json!({"url": "https://mcp.example.com/sse"}),
        confirmation_token: None,
    }
}

/// Parks a `ui/probe` request the way a Debug API client does and takes it like the UI does; returns its id and
/// capability (the request stays pending until answered or timed out).
async fn served(host: &Arc<Host>) -> (String, String, tokio::task::JoinHandle<()>) {
    let bridge = host.ui().clone();
    let waiting = tokio::spawn(async move {
        let _ = bridge.request("probe", Value::Null).await;
    });
    let request = host
        .ui()
        .next_request(Duration::from_secs(2))
        .await
        .expect("the request reached the UI");
    assert_eq!(request.capability.len(), 64, "256 bits as hex");
    (request.id, request.capability, waiting)
}

fn assert_refused(result: &Result<CommandResult, vw_core_api::CoreError>, case: &str) {
    let error = result.as_ref().expect_err(case);
    assert_eq!(error.code, ErrorCode::Denied, "{case}: {error:?}");
}

#[tokio::test]
async fn the_debug_api_origin_needs_the_capability_of_a_pending_request() {
    let dir = scratch("host-uic-origin");
    let mut supervisor = SupervisorConfig::new(fake_bin(), dir.join("vibe-home"), dir.clone());
    supervisor.extra_args = vec!["--script".into(), "--status-events".into()];
    supervisor.shutdown_grace = Duration::from_secs(1);
    let confirmer = common::host_fakes::Scripted::default();
    let host = Arc::new(
        Host::start(HostConfig {
            launch: Ok(supervisor),
            secrets: Arc::new(NoSecret),
            core: CoreConfig::default(),
            settings_path: dir.join("settings.json"),
            descriptor_dir: dir.join("debug-api"),
            dev_build: true,
            ui_reply_timeout: Duration::from_millis(600),
            picked_folders: Vec::new(),
            confirmer: confirmer.confirmer(),
            notifier: Arc::new(common::host_fakes::Recording::default()),
            keys: None,
            app_window: None,
        })
        .await,
    );
    let CommandResult::Session { snapshot, .. } = host
        .execute(Command::StartSession { agent_config: None })
        .await
        .unwrap()
    else {
        panic!("expected a session");
    };
    let sid = snapshot.session_id;
    confirmer.approve(false);

    // A real request: acts without a dialog.
    let (id, capability, waiting) = served(&host).await;
    host.execute_for_ui_control(shell(&sid), Some(&capability))
        .await
        .expect("the served request's command runs without a dialog");
    // The fake may answer `mcp/add` with an error of its own; only a gate refusal would be `confirmationRequired`.
    let on = host.execute_for_ui_control(add(), Some(&capability)).await;
    assert!(
        on.as_ref().err().is_none_or(
            |e| e.code != ErrorCode::ConfirmationRequired && e.code != ErrorCode::Denied
        ),
        "{on:?}"
    );
    assert!(confirmer.asked().is_empty(), "{:?}", confirmer.asked());

    // No capability, a made-up one: refused while that request is still pending.
    assert_refused(&host.execute_for_ui_control(add(), None).await, "none");
    let made_up = "0".repeat(64);
    assert_refused(
        &host.execute_for_ui_control(add(), Some(&made_up)).await,
        "made up",
    );
    assert_refused(&host.execute_for_ui_control(add(), Some("")).await, "empty");

    // Answered: the capability is dead.
    assert!(host.ui().reply(&id, UiOutcome::Ok(Value::Null)));
    waiting.await.unwrap();
    assert_refused(
        &host.execute_for_ui_control(add(), Some(&capability)).await,
        "answered",
    );

    // Past its deadline (600 ms): dead too.
    let (_late_id, late, late_waiting) = served(&host).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_refused(
        &host.execute_for_ui_control(add(), Some(&late)).await,
        "timed out",
    );
    late_waiting.await.unwrap();
    assert!(
        confirmer.asked().is_empty(),
        "a refused claim never reached a dialog: {:?}",
        confirmer.asked()
    );

    // The Debug API off: even a live capability buys nothing; the window's own rules apply.
    let (_off_id, live, off_waiting) = served(&host).await;
    host.set_debug_api(false).await.unwrap();
    assert_refused(
        &host.execute_for_ui_control(add(), Some(&live)).await,
        "Debug API off",
    );
    off_waiting.abort();
    host.execute(shell(&sid))
        .await
        .expect("the window's own origin keeps the trusted shell");
    host.shutdown().await;
}
