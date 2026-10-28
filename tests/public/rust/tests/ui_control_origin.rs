//! Window commands and the Debug API origin through a pinned real Vibe host and local scripted model: the window holds no Debug API authority.
//! Every window command runs as the window's, also while a Debug API `ui/*` request is served; only the exact
//! command a request's grant names runs as the Debug API, once, while that request is pending and the Debug API is on.
//! PREDICT: while a request without a grant waits, a window `mcp/add` still needs the window's confirmation and the
//! window's trusted shell still runs; a `ui/worktrees folder` request lets exactly its `pickFolder` run once, not
//! another path, not twice, not after the reply, not past the deadline, not with the Debug API off.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::processes::scratch;
use serde_json::{Value, json};
use vw_core_api::{Command, CommandResult, CoreConfig, ErrorCode};
use vw_host::{Host, HostConfig, UiOutcome};

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

fn pick(path: &std::path::Path) -> Command {
    Command::PickFolder {
        path: path.display().to_string(),
    }
}

/// Parks a `ui/<route>` request the way a Debug API client does and takes it like the UI does; returns its id (the
/// request stays pending until answered or timed out).
async fn served(
    host: &Arc<Host>,
    route: &'static str,
    body: Value,
) -> (String, tokio::task::JoinHandle<()>) {
    let bridge = host.ui().clone();
    let waiting = tokio::spawn(async move {
        let _ = bridge.request(route, body).await;
    });
    let request = host
        .ui()
        .next_request(Duration::from_secs(2))
        .await
        .expect("the request reached the UI");
    assert_eq!(request.name, route);
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(
        wire.as_object().unwrap().len(),
        3,
        "only id, name and body reach the page: {wire}"
    );
    (request.id, waiting)
}

fn assert_denied(result: &Result<CommandResult, vw_core_api::CoreError>, case: &str) {
    let error = result.as_ref().expect_err(case);
    assert_eq!(error.code, ErrorCode::Denied, "{case}: {error:?}");
}

#[tokio::test]
async fn window_commands_stay_the_windows_and_only_an_exact_grant_runs_as_the_debug_api() {
    let dir = scratch("host-uic-origin");
    let picked = dir.join("picked");
    let other = dir.join("other");
    std::fs::create_dir_all(&picked).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let model =
        common::scripted_model::ScriptedModel::start(common::real_vibe::text_script()).await;
    let mut supervisor = common::real_vibe::supervisor_config(&dir, &model);
    supervisor.shutdown_grace = Duration::from_secs(1);
    let confirmer = common::host_fakes::Scripted::default();
    let secret = model.route(&mut supervisor).await;
    let host = Arc::new(
        Host::start(HostConfig {
            launch: Ok(supervisor),
            secrets: secret,
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
    // Finish Vibe's lazy shell startup before opening the short-lived UI grant. The next shell
    // call is still checked while the ungranted request is pending, within the same deadline.
    host.execute_window(shell(&sid))
        .await
        .expect("initialize the real shell");
    confirmer.approve(false);

    // A request without a grant: the window's commands keep the window's rules.
    let (id, waiting) = served(&host, "click", json!({"selector": "#x"})).await;
    let raw = host.execute_window(add()).await.unwrap_err();
    assert_eq!(raw.code, ErrorCode::ConfirmationRequired, "{raw:?}");
    host.execute_window(shell(&sid))
        .await
        .expect("the window's trusted shell runs as before");
    assert_denied(&host.execute_window(pick(&picked)).await, "no grant");
    assert!(host.ui().reply(&id, UiOutcome::Ok(Value::Null)));
    waiting.await.unwrap();

    // `ui/worktrees folder {path}`: exactly that `pickFolder`, once.
    let body = json!({"action": "folder", "path": picked.display().to_string()});
    let (id, waiting) = served(&host, "worktrees", body.clone()).await;
    assert_denied(&host.execute_window(pick(&other)).await, "another path");
    host.execute_window(pick(&picked))
        .await
        .expect("the request's own pickFolder runs as the Debug API");
    assert_denied(&host.execute_window(pick(&picked)).await, "a second time");
    assert!(host.ui().reply(&id, UiOutcome::Ok(Value::Null)));
    waiting.await.unwrap();

    // Answered before use: the grant went with the request.
    let (id, waiting) = served(&host, "worktrees", body.clone()).await;
    assert!(host.ui().reply(&id, UiOutcome::Ok(Value::Null)));
    waiting.await.unwrap();
    assert_denied(&host.execute_window(pick(&picked)).await, "answered");

    // Past its deadline (600 ms): gone too.
    let (_late, late_waiting) = served(&host, "worktrees", body.clone()).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_denied(&host.execute_window(pick(&picked)).await, "timed out");
    late_waiting.await.unwrap();

    // The Debug API off: a pending request's grant buys nothing.
    let (_off, off_waiting) = served(&host, "worktrees", body).await;
    host.set_debug_api(false).await.unwrap();
    assert_denied(&host.execute_window(pick(&picked)).await, "Debug API off");
    off_waiting.abort();
    assert!(
        confirmer.asked().is_empty(),
        "no refused command reached a dialog: {:?}",
        confirmer.asked()
    );
    host.shutdown().await;
}
