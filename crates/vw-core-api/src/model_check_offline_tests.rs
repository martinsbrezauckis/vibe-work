//! Missing account catalogs with a usable key preserve defaults and both chats' choices.
//! Uses the parent stdio fixture and real pinned Vibe against a reserved, non-listening local port.

use super::{NoPrompt, check};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

const SAVED: &str = "mistral-medium-latest";

#[path = "model_check_offline_refresh_tests.rs"]
mod refreshed;

fn missing_catalog() -> Value {
    let large = json!({"alias":"mistral-large-4-0","name":"mistral-large-4-0",
        "provider":"mistral","source":"local","thinking":"off"});
    json!({"models":[large.clone()],"activeModel":large,
        "validationWarnings":["Model 'mistral-medium-latest' is not available"]})
}

async fn keys(root: &Path) -> Arc<vw_keys::KeyService> {
    let keys = crate::refresh_notice_tests::memory_keys(root).await;
    keys.store_key(vw_vault::SecretString::new(
        "offlinecatalog-test-key-000000000000",
    ))
    .await
    .unwrap();
    assert!(keys.speech_available());
    keys
}

async fn boot(
    root: &Path,
    supervisor: vw_supervisor::SupervisorConfig,
    keys: Arc<vw_keys::KeyService>,
) -> crate::Core {
    let sessions = root.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let record = json!({"remembered":{"alias":SAVED,"name":SAVED,"effort":"high",
        "provider":"mistral","provider_key":"offlinecatalog-saved-route"}});
    std::fs::write(
        sessions.join("model-choices.json"),
        json!({"version":1,"chats":{"one":record,"two":record}}).to_string(),
    )
    .unwrap();
    Box::pin(crate::Core::start(
        supervisor,
        keys.secret_provider(),
        NoPrompt,
        crate::CoreConfig {
            keys: Some(keys),
            sessions_dir: Some(sessions),
            instructions: crate::InstructionsConfig {
                cli_home: Some(root.join("cli")),
            },
            ..crate::CoreConfig::default()
        },
    ))
    .await
    .unwrap()
}

async fn preserve_two_chats(core: &crate::Core, root: &Path, config: Value) {
    assert!(super::super::account_models_unavailable(
        &core.shared,
        &config
    ));
    let file = root.join("sessions/model-choices.json");
    let before = std::fs::read(&file).unwrap();
    let mut events = core.subscribe(None);
    let runtime = json!({"config":config});
    let (one, two) = tokio::join!(
        crate::model_choices::restore(&core.shared, "one", runtime.clone(), None),
        crate::model_choices::restore(&core.shared, "two", runtime.clone(), None)
    );
    assert_eq!(one, runtime);
    assert_eq!(two, runtime);
    assert_eq!(
        std::fs::read(file).unwrap(),
        before,
        "saved picks/fallback changed"
    );
    let events = crate::refresh_notice_tests::drain_events(&mut events).await;
    assert!(
        !events.iter().any(|event| matches!(
            event,
            crate::CoreEvent::Alert {
                kind: crate::AlertKind::ModelChoiceNotRestored,
                ..
            }
        )),
        "missing catalog raised a model problem: {events:?}"
    );
}

#[tokio::test]
async fn usable_key_missing_catalog_is_undecided_and_preserves_default_and_two_chats() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("vibe");
    std::fs::create_dir_all(&home).unwrap();
    let original = "enable_model_catalog = true\nactive_model = 'mistral-medium-latest'\n";
    std::fs::write(home.join("config.toml"), original).unwrap();
    let program = root.path().join("lobby.py");
    std::fs::write(&program, super::LOBBY).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let keys = keys(root.path()).await;
    let core = boot(
        root.path(),
        vw_supervisor::SupervisorConfig::new(program, home.clone(), root.path().to_path_buf()),
        keys,
    )
    .await;
    for rows in [
        missing_catalog()["models"].clone(),
        json!([]),
        json!(null),
        json!({}),
        json!([{"alias":"private","source":"project"}]),
    ] {
        let mut config = missing_catalog();
        config["models"] = rows;
        std::fs::write(home.join("reply.json"), config.to_string()).unwrap();
        assert_eq!(check(&core).await["gone"], Value::Null, "{config}");
        preserve_two_chats(&core, root.path(), config).await;
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            original
        );
    }
    let mut config = missing_catalog();
    config.as_object_mut().unwrap().remove("models");
    std::fs::write(home.join("reply.json"), config.to_string()).unwrap();
    assert_eq!(check(&core).await["gone"], Value::Null);
    preserve_two_chats(&core, root.path(), config.clone()).await;
    // A later populated account list is authoritative, even if it omits the saved model.
    for source in ["catalog", "growthbook"] {
        config["models"] = json!([{"alias":"other","source":source}]);
        std::fs::write(home.join("reply.json"), config.to_string()).unwrap();
        assert_eq!(check(&core).await["gone"], SAVED);
    }
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        original
    );
    core.shutdown().await;
}

#[tokio::test]
async fn usable_key_missing_catalog_preserves_owned_facts() {
    let root = tempfile::tempdir().unwrap();
    let keys = keys(root.path()).await;
    let (core, root, home) = super::start_with_keys("offlinecatalog-facts", Some(keys)).await;
    let file = home.join("config.toml");
    let (original, written) = crate::model_facts::with(
        "active_model = 'mistral-medium-latest'\n",
        "mistral-large-4-0-3",
        &json!({}),
    );
    crate::model_facts::record(&home, written.as_ref()).unwrap();
    std::fs::write(&file, &original).unwrap();
    std::fs::write(home.join("reply.json"), missing_catalog().to_string()).unwrap();
    let result = check(&core).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    assert_eq!(crate::model_facts::recorded(&home), written);
    assert_eq!(result["gone"], Value::Null);
    core.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn pinned_315_closed_catalog_port_preserves_default_and_two_saved_chats() {
    for policy in ["ask", "accept-edits", "auto-approve"] {
        Box::pin(closed_catalog(policy, false)).await;
    }
}

#[tokio::test]
async fn pinned_315_stalled_catalog_waits_for_refresh_before_checking_failure() {
    Box::pin(closed_catalog("ask", true)).await;
}

async fn closed_catalog(policy: &str, stall_catalog: bool) {
    let delayed = if stall_catalog {
        Some(refreshed::DelayedCatalog::start().await)
    } else {
        None
    };
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    socket
        .bind(
            &"127.0.0.1:0"
                .parse::<std::net::SocketAddr>()
                .unwrap()
                .into(),
        )
        .unwrap();
    let closed = socket.local_addr().unwrap().as_socket().unwrap();
    eprintln!("PINNED_315_CATALOG_CLOSED_ENDPOINT={closed} policy={policy}");
    assert!(
        std::net::TcpStream::connect(closed).is_err(),
        "fixture port is not closed"
    );
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("vibe");
    std::fs::create_dir_all(&home).unwrap();
    let original = format!(
        "enable_telemetry = false\nenable_otel = false\nenable_connectors = false\n\
        enable_model_catalog = true\nactive_model = '{SAVED}'\ndefault_approval_policy = '{policy}'\n\
        [[providers]]\nname = 'mistral'\napi_base = 'http://{closed}/v1'\n\
        api_key_env_var = 'MISTRAL_API_KEY'\nbackend = 'mistral'\n"
    );
    std::fs::write(home.join("config.toml"), &original).unwrap();
    let keys = keys(root.path()).await;
    let supervisor = refreshed::supervisor(root.path(), home.clone(), closed, delayed.as_ref());
    let core = boot(root.path(), supervisor, keys).await;
    let reply = core
        .shared
        .gate
        .call("config/read", json!({}), None)
        .await
        .unwrap()
        .result;
    assert_eq!(reply["config"]["awaitingExperimentModel"], false, "{reply}");
    assert_eq!(reply["config"]["defaultApprovalPolicy"], policy);
    if let Some(catalog) = &delayed {
        catalog.wait_for_request().await;
        eprintln!("PINNED_315_LAUNCH_READ_RETURNED_WITH_CATALOG_RESPONSE_HELD");
        catalog.release();
    }
    // The lobby read caps its wait while its catalog fetch can continue. A completed
    // session reload settles that fetch; only then does the failure diagnostic belong here.
    Box::pin(refreshed::preserve_after_refresh(
        &core,
        root.path(),
        policy,
    ))
    .await;
    let log = std::fs::read_to_string(home.join("logs/vibe.log")).unwrap();
    assert!(
        log.contains("Model catalog request failed:"),
        "Vibe did not report a failed catalog request"
    );
    assert!(
        reply["config"]["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["source"] != "catalog")
    );
    assert_eq!(
        crate::model_choices::unavailable_default_alias(&reply["config"]).as_deref(),
        Some(SAVED),
        "{reply}"
    );
    assert_eq!(check(&core).await["gone"], Value::Null);
    preserve_two_chats(&core, root.path(), reply["config"].clone()).await;
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        original
    );
    core.shutdown().await;
}
