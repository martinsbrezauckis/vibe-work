//! Real pinned 3.1.4 reopen regression over a disposable project and Vibe home.
//! No model call or credential is needed: selection and reopen exercise the app server's config RPCs.
#![allow(clippy::unwrap_used)]

use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

#[cfg(unix)]
#[path = "unlock_runtime_tests.rs"]
mod unlock_runtime_tests;

struct NoPrompt;
impl Confirmer for NoPrompt {
    async fn confirm(&self, _: &ConfirmationRequest) -> Confirmation {
        panic!("model selection and reopen must not prompt");
    }
}

fn server() -> PathBuf {
    let python = std::env::var_os("VIBE_PYTHON").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../data")
                .join(format!(
                    "vibe-{}",
                    include_str!("../../../../tools/public/codegen/vibe.pin").trim()
                ))
                .join("venv/bin/python")
        },
        PathBuf::from,
    );
    python.with_file_name("vibe-app-server")
}

fn core_config(root: &Path, work: &Path) -> vw_supervisor::SupervisorConfig {
    let mut config =
        vw_supervisor::SupervisorConfig::new(server(), root.join("vibe-home"), work.to_path_buf());
    config.home = Some(root.join("home"));
    config.parent_env = Some(vec![
        (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
        (OsString::from("LANG"), OsString::from("C.UTF-8")),
    ]);
    config.extra_env.push((
        "PYTHON_KEYRING_BACKEND".into(),
        "keyring.backends.null.Keyring".into(),
    ));
    config
}

async fn start(root: &Path, work: &Path) -> crate::Core {
    Box::pin(crate::Core::start(
        core_config(root, work),
        Arc::new(vw_supervisor::NoSecret),
        NoPrompt,
        crate::CoreConfig {
            instructions: crate::InstructionsConfig {
                cli_home: Some(root.join("cli")),
            },
            sessions_dir: Some(root.join("sessions")),
            ..crate::CoreConfig::default()
        },
    ))
    .await
    .unwrap()
}

async fn call(core: &crate::Core, method: &str, params: Value) -> Value {
    let answer = core
        .execute(
            crate::Origin::Ui,
            crate::Command::Call {
                method: method.into(),
                params,
                confirmation_token: None,
            },
        )
        .await
        .unwrap();
    let crate::CommandResult::Reply { result, .. } = answer else {
        panic!("expected RPC reply")
    };
    result
}

#[tokio::test]
async fn pinned_314_project_provider_change_notices_on_reopen() {
    for policy in ["ask", "accept-edits", "auto-approve"] {
        provider_change_case(policy).await;
    }
}

#[allow(clippy::too_many_lines)] // One disposable server covers both chats and the reopen.
async fn provider_change_case(policy: &str) {
    let base = std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
    std::fs::create_dir_all(&base).unwrap();
    let root = tempfile::tempdir_in(base).unwrap();
    let work = root.path().join("work");
    let project = work.join(".vibe");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(root.path().join("vibe-home")).unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    let home_config = "enable_telemetry = false\nenable_otel = false\n\
        enable_connectors = false\nexperimental_enable_model_catalog = false\n\
        active_model = 'default'\ndefault_approval_policy = 'ask'\n\
        [[models]]\nalias = 'default'\nname = 'default-name'\nprovider = 'llamacpp'\n\
        [[providers]]\nname = 'llamacpp'\napi_base = 'http://127.0.0.1:19462/v1'\n\
        backend = 'generic'\n\
        [[providers]]\nname = 'mistral'\napi_base = 'http://127.0.0.1:19463/v1'\n\
        backend = 'mistral'\n";
    std::fs::write(
        root.path().join("vibe-home/config.toml"),
        home_config.replace(
            "default_approval_policy = 'ask'",
            &format!("default_approval_policy = '{policy}'"),
        ),
    )
    .unwrap();
    let project_model = |provider: &str| {
        format!(
            "[[models]]\nalias = 'same'\nname = 'same-name'\nprovider = '{provider}'\nthinking = 'high'\n"
        )
    };
    std::fs::write(project.join("config.toml"), project_model("llamacpp")).unwrap();

    let core = start(root.path(), &work).await;
    core.register_picked_folder(&work).unwrap();
    let agent_config = |cwd: &Path| {
        if policy == "auto-approve" {
            json!({"cwd":cwd,"trustWorkspace":true})
        } else {
            json!({"cwd":cwd,"approvalPolicy":policy,"trustWorkspace":true})
        }
    };
    let session = core
        .execute(
            crate::Origin::Ui,
            crate::Command::StartSession {
                agent_config: Some(agent_config(&work)),
            },
        )
        .await
        .unwrap();
    let crate::CommandResult::Session { snapshot, .. } = session else {
        panic!("expected session")
    };
    let id = snapshot.session_id;
    let before = call(&core, "runtime/read", json!({"sessionId":id})).await;
    assert_eq!(before["runtime"]["activeApprovalPolicy"]["name"], policy);
    assert!(
        before["runtime"]["config"]["activeModel"]
            .get("provider")
            .is_none()
    );
    let selected = core
        .execute(
            crate::Origin::Ui,
            crate::Command::PickModel {
                model_name: None,
                model_display_name: None,
                session_id: id.clone(),
                model_alias: Some("same".into()),
                reasoning_effort: Some("high".into()),
            },
        )
        .await
        .unwrap();
    let crate::CommandResult::Reply { result, .. } = selected else {
        panic!("expected pick")
    };
    assert_ne!(result["modelChoiceFallback"], true, "{result}");
    let stored: Value = serde_json::from_slice(
        &std::fs::read(root.path().join("sessions/model-choices.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(stored["chats"][&id]["remembered"]["provider"], "llamacpp");
    assert!(
        stored["chats"][&id]["remembered"]["provider_key"]
            .as_str()
            .is_some_and(|key| !key.is_empty())
    );

    // Two open chats with the same alias/name must resolve against their own projects.
    let other_work = root.path().join("work-other");
    let other_project = other_work.join(".vibe");
    std::fs::create_dir_all(&other_project).unwrap();
    std::fs::write(other_project.join("config.toml"), project_model("mistral")).unwrap();
    core.register_picked_folder(&other_work).unwrap();
    let other = core
        .execute(
            crate::Origin::Ui,
            crate::Command::StartSession {
                agent_config: Some(agent_config(&other_work)),
            },
        )
        .await
        .unwrap();
    let crate::CommandResult::Session { snapshot, .. } = other else {
        panic!("expected second session")
    };
    let other_id = snapshot.session_id;
    let other_pick = core
        .execute(
            crate::Origin::Ui,
            crate::Command::PickModel {
                model_name: None,
                model_display_name: None,
                session_id: other_id.clone(),
                model_alias: Some("same".into()),
                reasoning_effort: Some("high".into()),
            },
        )
        .await
        .unwrap();
    let crate::CommandResult::Reply { result, .. } = other_pick else {
        panic!("expected second pick")
    };
    assert_ne!(result["modelChoiceFallback"], true, "{result}");
    let stored: Value = serde_json::from_slice(
        &std::fs::read(root.path().join("sessions/model-choices.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(stored["chats"][&id]["remembered"]["provider"], "llamacpp");
    assert_eq!(
        stored["chats"][&other_id]["remembered"]["provider"],
        "mistral"
    );

    std::fs::write(project.join("config.toml"), project_model("mistral")).unwrap();
    let mut events = core.subscribe(Some(id.clone()));
    call(
        &core,
        "config/reload",
        json!({"sessionId":id,"reloadRuntime":true}),
    )
    .await;
    core.execute(
        crate::Origin::Ui,
        crate::Command::ResumeSession {
            session_id: id.clone(),
            agent_config: None,
        },
    )
    .await
    .unwrap();
    let current = call(&core, "runtime/read", json!({"sessionId":id})).await;
    let alias = current["runtime"]["config"]["activeModel"]["alias"]
        .as_str()
        .unwrap();
    assert_ne!(
        alias, "same",
        "backend changed without a visible fallback: {current}"
    );
    let mut notices = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(300), events.next()).await
    {
        if let crate::CoreEvent::Alert {
            kind: crate::AlertKind::ModelChoiceNotRestored,
            message,
            ..
        } = event
        {
            notices.push(message);
        }
    }
    assert_eq!(notices.len(), 1, "provider switch notice: {notices:?}");
    let other_runtime = call(&core, "runtime/read", json!({"sessionId":other_id})).await;
    assert_eq!(
        other_runtime["runtime"]["config"]["activeModel"]["alias"],
        "same"
    );
    eprintln!(
        "PINNED_314_PROVIDER_SWITCH policy={policy} picked=llamacpp reopened_alias={alias} notices={} other_chat=same",
        notices.len()
    );
    core.shutdown().await;
}
