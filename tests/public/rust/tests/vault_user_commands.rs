//! Vault command behavior through the public core API and disposable fake services.
//! PREDICT: edits preserve identity, generated values stay one-way, and locked access needs a user password.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::fake::{fake_config, scratch};
use common::gate::FakeConfirmer;
use common::vault::key_service_config;
use serde_json::{Value, json};
use vw_core_api::{Command, CommandResult, Core, CoreConfig, Origin};
use vw_gate::Confirmation;
use vw_keys::KeyService;
use vw_keys::existing::MemoryVibeKeyring;
use vw_platform::clipboard::{ClipboardBackend, ClipboardError, ClipboardService};
use vw_vault::command_grant::{CommandGrant, id_for};
use vw_vault::fill_grant::{FillGrant, id_for as fill_id_for};
use vw_vault::items::ResourcePermission;
use vw_vault::location::VaultLocation;
use vw_vault::mcp_binding::McpBinding;
use vw_vault::{SecretString, Vault, VaultConfig};

const FIRST: &str = "toy-synthetic-secret-111";
const SECOND: &str = "toy-synthetic-secret-222";
const LOGIN_FIRST: &str = " toy-login-password-111 ";
const LOGIN_SECOND: &str = " toy-login-password-222 ";

struct Clipboard(Arc<Mutex<String>>);

impl ClipboardBackend for Clipboard {
    fn set_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        *self.0.lock().unwrap() = text.to_owned();
        Ok(())
    }
    fn get_text(&mut self) -> Result<String, ClipboardError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn clear(&mut self) -> Result<(), ClipboardError> {
        self.0.lock().unwrap().clear();
        Ok(())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

async fn run(core: &Core, op: Value) -> Value {
    let command: Command = serde_json::from_value(json!({"type":"secrets","request":op})).unwrap();
    let serialized = serde_json::to_string(&command).unwrap();
    let debug = format!("{command:?}");
    for value in [FIRST, SECOND, LOGIN_FIRST, LOGIN_SECOND] {
        assert!(!serialized.contains(value));
        assert!(!debug.contains(value));
    }
    let result = core.execute(Origin::Ui, command).await.unwrap();
    let CommandResult::Secrets { result } = result else {
        panic!("wrong result")
    };
    serde_json::to_value(result).unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn edits_keep_ids_grants_and_metadata_and_copy_returns_only_done() {
    let pass = format!("test-passphrase-{}", vw_vault::login::new_id());
    let (log, _guard) = common::logs::capture();
    let fake = common::fake_mistral::start().await;
    let dir = scratch("toy-vault-commands");
    let service = KeyService::start(key_service_config(
        &dir,
        &fake.base(),
        Arc::new(MemoryVibeKeyring::default()),
    ))
    .await
    .unwrap();
    let clipboard_state = Arc::new(Mutex::new(String::new()));
    let clipboard = Arc::new(
        ClipboardService::with_backend(
            Box::new(Clipboard(clipboard_state.clone())),
            Duration::from_secs(30),
        )
        .unwrap(),
    );
    let mut config = CoreConfig::default();
    config.keys = Some(service.clone());
    config.clipboard = Some(clipboard);
    let mut supervisor = fake_config("toy-vault-core", &["--script"]);
    supervisor.vibe_home = dir.join("vibe-home");
    std::fs::create_dir_all(&supervisor.vibe_home).unwrap();
    let confirmer = FakeConfirmer::new();
    let core = Core::start(
        supervisor,
        service.secret_provider(),
        confirmer.clone(),
        config,
    )
    .await
    .unwrap();
    let mut events = core.subscribe(None);
    core.execute(
        Origin::Ui,
        serde_json::from_value(json!({"type":"keys","request":
        {"op":"createVault","passphrase":pass.as_str(),"rememberDevice":false}}))
        .unwrap(),
    )
    .await
    .unwrap();

    let added = run(
        &core,
        json!({"op":"add","name":"Old key","value":FIRST,"label":"mcp"}),
    )
    .await;
    let id = added["secret"]["id"].as_str().unwrap().to_owned();
    let captured = service
        .store_secret_labelled(
            "Old key".to_owned(),
            SecretString::new(FIRST),
            Some("https://source.example.com".to_owned()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(captured.secret.id, id);
    let login = run(
        &core,
        json!({"op":"addLogin","name":"Old login","url":"https://example.com/signin",
        "username":"ada","password":LOGIN_FIRST}),
    )
    .await;
    let login_id = login["login"]["id"].as_str().unwrap().to_owned();
    let project = "/tmp/toy-project".to_owned();
    let fingerprint = "f".repeat(64);
    let grant_id = id_for(&project, &fingerprint);
    let grant = CommandGrant {
        id: grant_id.clone(),
        project,
        fingerprint,
        command: "tool".to_owned(),
        bindings: vec!["TOKEN <- Old key".to_owned()],
        secret_ids: vec![id.clone()],
        always: true,
        granted_ms: now_ms(),
        revoked_ms: 0,
        created_ms: now_ms(),
    };
    service
        .update_command_grant(grant_id.clone(), move |_| Some(grant.clone()))
        .await
        .unwrap();
    let binding = McpBinding {
        id: vw_vault::mcp_binding::new_id(),
        server: "one".to_owned(),
        upstream: "https://mcp.example.com/mcp".to_owned(),
        header: "authorization".to_owned(),
        bearer: true,
        secret_id: id.clone(),
        secret_name: "Old key".to_owned(),
        always: true,
        revoked_ms: 0,
        created_ms: now_ms(),
    };
    service.store_mcp_binding(binding.clone()).await.unwrap();
    let origin = "https://example.com";
    let fill_id = fill_id_for(&login_id, origin);
    let fill = FillGrant {
        id: fill_id.clone(),
        login_id: login_id.clone(),
        login_name: "Old login".to_owned(),
        origin: origin.to_owned(),
        always: true,
        granted_ms: now_ms(),
        revoked_ms: 0,
        created_ms: now_ms(),
    };
    service
        .update_fill_grant(fill_id, move |_| Some(fill.clone()))
        .await
        .unwrap();

    confirmer.then(Confirmation::Declined);
    let replacement: Command = serde_json::from_value(json!({"type":"secrets","request":{
        "op":"replace", "id":id, "value":SECOND
    }}))
    .unwrap();
    let cancelled = core.execute(Origin::Ui, replacement).await.unwrap_err();
    assert_eq!(cancelled.code, vw_core_api::ErrorCode::ConfirmationDeclined);
    assert_eq!(
        service
            .listed_secret_value(id.clone())
            .await
            .unwrap()
            .expose(),
        FIRST
    );

    for op in [
        json!({"op":"rename","id":id,"name":"New key"}),
        json!({"op":"replace","id":id,"value":SECOND}),
        json!({"op":"renameLogin","id":login_id,"name":"New login"}),
        json!({"op":"replaceLoginPassword","id":login_id,"password":LOGIN_SECOND}),
    ] {
        assert_eq!(run(&core, op).await, json!({"kind":"done"}));
    }
    assert!(
        confirmer
            .shown()
            .iter()
            .any(|r| r.method == "vibe-work/replaceVaultSecret")
    );
    assert!(
        confirmer
            .shown()
            .iter()
            .any(|r| r.method == "vibe-work/replaceVaultLoginPassword")
    );
    let listed = run(&core, json!({"op":"list"})).await;
    assert_eq!(listed["secrets"][0]["id"], id);
    assert_eq!(listed["secrets"][0]["name"], "New key");
    assert_eq!(listed["secrets"][0]["label"], "mcp");
    assert_eq!(
        listed["secrets"][0]["sourceOrigin"],
        "https://source.example.com"
    );
    assert_eq!(listed["secrets"][0]["fields"], json!(["value"]));
    assert_eq!(listed["grants"][0]["id"], grant_id);
    assert_eq!(listed["grants"][0]["bindings"][0], "TOKEN <- New key");
    assert_eq!(listed["serverGrants"][0]["id"], binding.id);
    assert_eq!(listed["serverGrants"][0]["secretName"], "New key");
    assert_eq!(listed["logins"][0]["id"], login_id);
    assert_eq!(listed["logins"][0]["name"], "New login");
    assert_eq!(listed["fillGrants"][0]["loginName"], "New login");
    let text = listed.to_string();
    for value in [FIRST, SECOND, LOGIN_FIRST, LOGIN_SECOND] {
        assert!(!text.contains(value));
    }
    run(
        &core,
        json!({"op":"add","name":"Other owned","value":"toy-other-owned-555"}),
    )
    .await;
    let conflict: Command = serde_json::from_value(json!({"type":"secrets","request":
        {"op":"rename","id":id,"name":"other OWNED"}}))
    .unwrap();
    assert!(
        core.execute(Origin::Ui, conflict).await.is_err(),
        "secret names remain unique without case"
    );

    for (op, expected) in [
        (json!({"op":"copy","id":id}), SECOND),
        (json!({"op":"copyLoginUsername","id":login_id}), "ada"),
        (
            json!({"op":"copyLoginPassword","id":login_id}),
            LOGIN_SECOND,
        ),
    ] {
        assert_eq!(run(&core, op).await, json!({"kind":"done"}));
        assert_eq!(*clipboard_state.lock().unwrap(), expected);
    }
    // Other Vault clients' listed items are copyable, while edits stay restricted to Vibe Work's items.
    let other = Vault::new(VaultConfig {
        location: VaultLocation::explicit(dir.join("shellx-vault")),
        device_store: common::vault::memory_store(),
        kdf: common::vault::fast_kdf(),
        device_name: "other-test-client".to_owned(),
    })
    .unwrap();
    other
        .unlock(&SecretString::new(pass.as_str()), None, None)
        .unwrap();
    let foreign_id = "other-client-secret-0001";
    let foreign_value = " toy-other-client-secret-333 ";
    let mut foreign = vw_vault::project_secret::resource(
        foreign_id,
        "Other key",
        &SecretString::new(foreign_value),
        None,
        None,
        None,
        now_ms(),
    );
    foreign
        .secret_fields
        .insert("value".to_owned(), foreign_value.to_owned());
    other.upsert_item(&foreign.to_item().unwrap()).unwrap();
    let hidden_id = "other-client-hidden-0001";
    let mut hidden = vw_vault::project_secret::resource(
        hidden_id,
        "Hidden key",
        &SecretString::new("toy-hidden-value-444"),
        None,
        None,
        None,
        now_ms(),
    );
    hidden.permission = ResourcePermission::UserOnly;
    other.upsert_item(&hidden.to_item().unwrap()).unwrap();
    let foreign_login_id = "other-client-login-0001";
    let mut foreign_login = vw_vault::login::item(
        foreign_login_id,
        &vw_vault::login::NewLogin {
            name: "Other login",
            url: "https://foreign.example.com",
            username: "other-user",
            password: &SecretString::exact("toy-other-login-password"),
        },
        None,
        now_ms(),
    )
    .unwrap();
    foreign_login.password = "x".to_owned();
    other.upsert_item(&foreign_login).unwrap();
    assert_eq!(
        run(&core, json!({"op":"copy","id":foreign_id})).await,
        json!({"kind":"done"})
    );
    assert_eq!(*clipboard_state.lock().unwrap(), foreign_value);
    assert_eq!(
        run(
            &core,
            json!({"op":"copyLoginPassword","id":foreign_login_id})
        )
        .await,
        json!({"kind":"done"})
    );
    assert_eq!(*clipboard_state.lock().unwrap(), "x");
    for op in [
        json!({"op":"rename","id":foreign_id,"name":"Changed"}),
        json!({"op":"replace","id":foreign_id,"value":SECOND}),
        json!({"op":"renameLogin","id":foreign_login_id,"name":"Changed"}),
        json!({"op":"replaceLoginPassword","id":foreign_login_id,"password":LOGIN_SECOND}),
        json!({"op":"copy","id":hidden_id}),
    ] {
        let command: Command =
            serde_json::from_value(json!({"type":"secrets","request":op})).unwrap();
        assert!(core.execute(Origin::Ui, command).await.is_err());
    }
    assert_eq!(*clipboard_state.lock().unwrap(), "x");
    let mut event_text = String::new();
    while let Some(event) = events.try_next() {
        event_text.push_str(&serde_json::to_string(&event).unwrap());
    }
    let trace = log.text();
    for value in [
        FIRST,
        SECOND,
        LOGIN_FIRST,
        LOGIN_SECOND,
        foreign_value,
        "toy-other-login-password",
        "toy-hidden-value-444",
    ] {
        assert!(!trace.contains(value), "TRACE included a vault value");
        assert!(
            !event_text.contains(value),
            "core event included a vault value"
        );
    }
    other.lock();
    core.shutdown().await;
    service.shutdown().await;
}

#[tokio::test(flavor = "current_thread")]
async fn generated_values_are_one_way_and_agent_access_is_user_only() {
    let pass = format!("test-passphrase-{}", vw_vault::login::new_id());
    let (log, _guard) = common::logs::capture();
    let fake = common::fake_mistral::start().await;
    let dir = scratch("new-generated-vault");
    let service = KeyService::start(key_service_config(
        &dir,
        &fake.base(),
        Arc::new(MemoryVibeKeyring::default()),
    ))
    .await
    .unwrap();
    let clipboard_state = Arc::new(Mutex::new(String::new()));
    let clipboard = Arc::new(
        ClipboardService::with_backend(
            Box::new(Clipboard(clipboard_state.clone())),
            Duration::from_secs(30),
        )
        .unwrap(),
    );
    let mut config = CoreConfig::default();
    config.keys = Some(service.clone());
    config.clipboard = Some(clipboard);
    let mut supervisor = fake_config("new-vault-core", &["--script"]);
    supervisor.vibe_home = dir.join("vibe-home");
    std::fs::create_dir_all(&supervisor.vibe_home).unwrap();
    let confirmer = FakeConfirmer::new();
    let core = Core::start(
        supervisor,
        service.secret_provider(),
        confirmer.clone(),
        config,
    )
    .await
    .unwrap();
    let mut events = core.subscribe(None);
    core.execute(
        Origin::Ui,
        serde_json::from_value(json!({"type":"keys","request":
        {"op":"createVault","passphrase":pass.as_str(),"rememberDevice":false}}))
        .unwrap(),
    )
    .await
    .unwrap();

    let options = json!({"length":32,"uppercase":true,"digits":true,"symbols":true});
    let added = run(
        &core,
        json!({"op":"generate","name":"Generated","options":options}),
    )
    .await;
    let id = added["secret"]["id"].as_str().unwrap().to_owned();
    let first = clipboard_state.lock().unwrap().clone();
    let mut generated = vec![first.clone()];
    assert_eq!(first.len(), 32);
    assert!(!added.to_string().contains(&first));
    let duplicate: Command = serde_json::from_value(json!({"type":"secrets","request":
        {"op":"generate","name":"generated"}}))
    .unwrap();
    assert!(core.execute(Origin::Ui, duplicate).await.is_err());
    assert_eq!(
        service
            .listed_secret_value(id.clone())
            .await
            .unwrap()
            .expose(),
        &first
    );

    confirmer.then(Confirmation::Declined);
    let cancelled: Command = serde_json::from_value(json!({"type":"secrets","request":
        {"op":"regenerate","id":id}}))
    .unwrap();
    let refused = core.execute(Origin::Ui, cancelled).await.unwrap_err();
    assert_eq!(refused.code, vw_core_api::ErrorCode::ConfirmationDeclined);
    assert_eq!(
        service
            .listed_secret_value(id.clone())
            .await
            .unwrap()
            .expose(),
        &first
    );
    assert_eq!(*clipboard_state.lock().unwrap(), first);
    assert_eq!(
        run(&core, json!({"op":"regenerate","id":id})).await,
        json!({"kind":"done"})
    );
    let second = clipboard_state.lock().unwrap().clone();
    generated.push(second.clone());
    assert_eq!(second.len(), 24, "default length");
    assert_ne!(first, second);
    assert_eq!(
        service
            .listed_secret_value(id.clone())
            .await
            .unwrap()
            .expose(),
        &second
    );
    assert_eq!(
        run(&core, json!({"op":"list"})).await["secrets"][0]["id"],
        id
    );

    for length in [7, 129] {
        let command: Command = serde_json::from_value(json!({"type":"secrets","request":
            {"op":"regenerate","id":id,"options":{"length":length}}}))
        .unwrap();
        assert!(core.execute(Origin::Ui, command).await.is_err());
        assert_eq!(
            service
                .listed_secret_value(id.clone())
                .await
                .unwrap()
                .expose(),
            &second
        );
    }
    for length in [8, 128] {
        run(
            &core,
            json!({"op":"regenerate","id":id,"options":
            {"length":length,"uppercase":false,"digits":false,"symbols":false}}),
        )
        .await;
        let value = clipboard_state.lock().unwrap().clone();
        assert_eq!(value.len(), length);
        assert!(value.bytes().all(|b| b.is_ascii_lowercase()));
        generated.push(value);
    }

    let login = run(
        &core,
        json!({"op":"generateLogin","name":"Example","url":"https://example.com",
        "username":"ada","options":{"length":16,"uppercase":false,"digits":true,"symbols":false}}),
    )
    .await;
    let login_id = login["login"]["id"].as_str().unwrap().to_owned();
    let login_first = clipboard_state.lock().unwrap().clone();
    generated.push(login_first.clone());
    assert_eq!(login_first.len(), 16);
    assert!(
        login_first
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    );
    let grant_id = id_for("/tmp/new-project", &"a".repeat(64));
    let command_grant = CommandGrant {
        id: grant_id.clone(),
        project: "/tmp/new-project".to_owned(),
        fingerprint: "a".repeat(64),
        command: "tool".to_owned(),
        bindings: vec!["TOKEN <- Generated".to_owned()],
        secret_ids: vec![id.clone()],
        always: true,
        granted_ms: now_ms(),
        revoked_ms: 0,
        created_ms: now_ms(),
    };
    service
        .update_command_grant(grant_id.clone(), move |_| Some(command_grant.clone()))
        .await
        .unwrap();
    let binding = McpBinding {
        id: vw_vault::mcp_binding::new_id(),
        server: "generated-key-server".to_owned(),
        upstream: "https://mcp.example.com/mcp".to_owned(),
        header: "authorization".to_owned(),
        bearer: true,
        secret_id: id.clone(),
        secret_name: "Generated".to_owned(),
        always: true,
        revoked_ms: 0,
        created_ms: now_ms(),
    };
    service.store_mcp_binding(binding.clone()).await.unwrap();
    let fill_id = fill_id_for(&login_id, "https://example.com");
    let fill = FillGrant {
        id: fill_id.clone(),
        login_id: login_id.clone(),
        login_name: "Example".to_owned(),
        origin: "https://example.com".to_owned(),
        always: true,
        granted_ms: now_ms(),
        revoked_ms: 0,
        created_ms: now_ms(),
    };
    service
        .update_fill_grant(fill_id.clone(), move |_| Some(fill.clone()))
        .await
        .unwrap();
    assert!(
        !login
            .to_string()
            .contains(&*clipboard_state.lock().unwrap())
    );
    let duplicate_login: Command = serde_json::from_value(json!({"type":"secrets","request":
        {"op":"generateLogin","name":"Other","url":"https://example.com/path","username":"ada"}}))
    .unwrap();
    assert!(core.execute(Origin::Ui, duplicate_login).await.is_err());
    confirmer.then(Confirmation::Declined);
    let cancelled_login: Command = serde_json::from_value(json!({"type":"secrets","request":
        {"op":"regenerateLoginPassword","id":login_id}}))
    .unwrap();
    assert_eq!(
        core.execute(Origin::Ui, cancelled_login)
            .await
            .unwrap_err()
            .code,
        vw_core_api::ErrorCode::ConfirmationDeclined
    );
    assert_eq!(
        service
            .login_password(login_id.clone())
            .await
            .unwrap()
            .expose(),
        login_first
    );
    assert_eq!(
        run(&core, json!({"op":"regenerateLoginPassword","id":login_id})).await,
        json!({"kind":"done"})
    );
    let login_second = clipboard_state.lock().unwrap().clone();
    generated.push(login_second.clone());
    assert_eq!(login_second.len(), 24);
    assert_ne!(login_first, login_second);
    run(&core, json!({"op":"regenerate","id":id})).await;
    generated.push(clipboard_state.lock().unwrap().clone());
    let listed = run(&core, json!({"op":"list"})).await;
    assert_eq!(listed["secrets"][0]["id"], id);
    assert_eq!(listed["logins"][0]["id"], login_id);
    assert_eq!(listed["grants"][0]["id"], grant_id);
    assert_eq!(listed["serverGrants"][0]["id"], binding.id);
    assert_eq!(listed["fillGrants"][0]["id"], fill_id);

    let lock: Command = serde_json::from_value(json!({"type":"secrets","request":
        {"op":"setAgentAccess","locked":true}}))
    .unwrap();
    let refused = core
        .execute(Origin::DebugApi, lock.clone())
        .await
        .unwrap_err();
    assert_eq!(refused.server_code.as_deref(), Some("vaultUserOnly"));
    core.execute(Origin::Ui, lock).await.unwrap();
    assert_eq!(
        run(&core, json!({"op":"list"})).await["agentAccessLocked"],
        true
    );
    let list: Command =
        serde_json::from_value(json!({"type":"secrets","request":{"op":"list"}})).unwrap();
    let refused = core.execute(Origin::DebugApi, list).await.unwrap_err();
    assert_eq!(refused.server_code.as_deref(), Some("agentAccessLocked"));
    let project_list: Command = serde_json::from_value(json!({"type":"secrets","request":
        {"op":"listForProject","project":"/synthetic/project"}}))
    .unwrap();
    let refused = core
        .execute(Origin::DebugApi, project_list)
        .await
        .unwrap_err();
    assert_eq!(refused.server_code.as_deref(), Some("agentAccessLocked"));
    assert_eq!(
        refused.message,
        "Not run: agent access to secrets is paused. There is no access to secrets until the user resumes it."
    );
    service.lock();
    let unlock: Command = serde_json::from_value(json!({"type":"secrets","request":
        {"op":"setAgentAccess","locked":false}}))
    .unwrap();
    let password = core.execute(Origin::Ui, unlock).await.unwrap_err();
    assert_eq!(password.server_code.as_deref(), Some("passwordRequired"));
    assert_eq!(
        run(&core, json!({"op":"list"})).await["agentAccessLocked"],
        true
    );
    service
        .unlock(SecretString::new(pass.as_str()), None, None)
        .await
        .unwrap();
    run(&core, json!({"op":"setAgentAccess","locked":false})).await;
    assert_eq!(
        run(&core, json!({"op":"list"})).await["agentAccessLocked"],
        false
    );
    run(&core, json!({"op":"setAgentAccess","locked":true})).await;
    let trace = log.text();
    let mut event_text = String::new();
    while let Some(event) = events.try_next() {
        event_text.push_str(&serde_json::to_string(&event).unwrap());
    }
    for value in &generated {
        assert!(!trace.contains(value), "trace included a generated value");
        assert!(
            !event_text.contains(value),
            "event included a generated value"
        );
    }
    core.shutdown().await;
    let mut config = CoreConfig::default();
    config.keys = Some(service.clone());
    config.clipboard = Some(Arc::new(
        ClipboardService::with_backend(
            Box::new(Clipboard(Arc::new(Mutex::new(String::new())))),
            Duration::from_secs(30),
        )
        .unwrap(),
    ));
    let mut supervisor = fake_config("new-restarted-core", &["--script"]);
    supervisor.vibe_home = dir.join("vibe-home");
    let restarted = Core::start(
        supervisor,
        service.secret_provider(),
        FakeConfirmer::new(),
        config,
    )
    .await
    .unwrap();
    assert_eq!(
        run(&restarted, json!({"op":"list"})).await["agentAccessLocked"],
        false
    );
    restarted.shutdown().await;
    service.shutdown().await;
}

#[tokio::test(flavor = "current_thread")]
async fn user_access_unlock_uses_remembered_key_then_requires_password_without_it() {
    let pass = format!("test-passphrase-{}", vw_vault::login::new_id());
    let fake = common::fake_mistral::start().await;
    let dir = scratch("new-remembered-access");
    let mut key_config =
        key_service_config(&dir, &fake.base(), Arc::new(MemoryVibeKeyring::default()));
    let device_store = common::vault::memory_store();
    key_config.vault.device_store = device_store.clone();
    let service = KeyService::start(key_config).await.unwrap();
    let mut config = CoreConfig::default();
    config.keys = Some(service.clone());
    config.clipboard = Some(Arc::new(
        ClipboardService::with_backend(
            Box::new(Clipboard(Arc::new(Mutex::new(String::new())))),
            Duration::from_secs(30),
        )
        .unwrap(),
    ));
    let mut supervisor = fake_config("new-remembered-core", &["--script"]);
    supervisor.vibe_home = dir.join("vibe-home");
    std::fs::create_dir_all(&supervisor.vibe_home).unwrap();
    let core = Core::start(
        supervisor,
        service.secret_provider(),
        FakeConfirmer::new(),
        config,
    )
    .await
    .unwrap();
    core.execute(
        Origin::Ui,
        serde_json::from_value(json!({"type":"keys","request":
        {"op":"createVault","passphrase":pass.as_str(),"rememberDevice":true}}))
        .unwrap(),
    )
    .await
    .unwrap();
    run(&core, json!({"op":"setAgentAccess","locked":true})).await;
    service.lock();
    assert_eq!(run(&core, json!({"op":"list"})).await["unlocked"], false);
    let background = common::vault::client(&dir.join("shellx-vault"), device_store.clone());
    background
        .unlock(&SecretString::new(pass.as_str()), None, None)
        .unwrap();
    background.lock();
    assert!(
        !background.unlock_remembered().unwrap(),
        "background unlock honors manual lock"
    );
    run(&core, json!({"op":"setAgentAccess","locked":false})).await;
    assert_eq!(run(&core, json!({"op":"list"})).await["unlocked"], true);
    service
        .unlock(SecretString::new(pass.as_str()), Some(false), None)
        .await
        .unwrap();
    run(&core, json!({"op":"setAgentAccess","locked":true})).await;
    service.lock();
    let unlock: Command = serde_json::from_value(json!({"type":"secrets","request":
        {"op":"setAgentAccess","locked":false}}))
    .unwrap();
    let error = core.execute(Origin::Ui, unlock).await.unwrap_err();
    assert_eq!(error.server_code.as_deref(), Some("passwordRequired"));
    service
        .unlock(SecretString::new(pass.as_str()), Some(true), None)
        .await
        .unwrap();
    run(&core, json!({"op":"setAgentAccess","locked":false})).await;
    let listed = run(&core, json!({"op":"list"})).await;
    assert_eq!(listed["agentAccessLocked"], false);
    assert_eq!(listed["unlocked"], true);
    core.shutdown().await;
    service.shutdown().await;
    let mut restarted =
        key_service_config(&dir, &fake.base(), Arc::new(MemoryVibeKeyring::default()));
    restarted.vault.device_store = device_store;
    let service = KeyService::start(restarted).await.unwrap();
    assert!(
        service.project_secrets().await.is_ok(),
        "remembered startup unlocks the vault"
    );
    service.shutdown().await;
}
