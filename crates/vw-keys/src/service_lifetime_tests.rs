//! Isolated key-service startup and teardown regressions. Uses disposable vaults and loopback listeners only.
//! Called by the crate's unit tests; never opens an OS credential store or a provider login.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::service_start::start_clients;
use super::*;
use vw_mistral::network::clear_system_proxy_if;

static LIFETIME_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn config(dir: &std::path::Path) -> KeyServiceConfig {
    vw_vault::location::forbid_canonical_profile();
    KeyServiceConfig {
        managed_vault: None,
        vault: VaultConfig {
            location: vw_vault::location::VaultLocation::explicit(dir.join("vault")),
            device_store: Arc::new(vw_vault::remember::MemoryDeviceStore::default()),
            kdf: vw_vault::KdfParams::default(),
            device_name: "relay-lifetime-test".into(),
        },
        proxy: ProxyConfig::default(),
        test_api_base: "http://127.0.0.1:1".into(),
        settings_path: dir.join("keys.json"),
        existing: ExistingKeys {
            dotenv: dir.join("vibe-home/.env"),
            keyring: Arc::new(crate::existing::MemoryVibeKeyring::default()),
        },
    }
}

async fn released(address: std::net::SocketAddr) -> bool {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if std::net::TcpListener::bind(address).is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

#[tokio::test]
async fn service_drop_closes_owned_forwarders() {
    let _serial = LIFETIME_TEST.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let service = KeyService::start(config(dir.path())).await.unwrap();
    let core = service.network_forwarder.clone();
    let vibe = service.vibe_forwarder.clone();
    drop(service);
    let core_released = released(core.addr()).await;
    let vibe_released = released(vibe.addr()).await;
    // Clean the base implementation's leaked listeners even when the red assertion fails.
    core.set_enabled(false);
    vibe.set_enabled(false);
    core.shutdown();
    vibe.shutdown();
    assert!(
        core_released && vibe_released,
        "dropped service kept its relay listeners"
    );
}

#[tokio::test]
async fn startup_failures_shutdown_and_old_owner_preserve_new_service() {
    let _serial = LIFETIME_TEST.lock().await;
    for stage in ["proxy start", "client slot"] {
        let endpoint = format!("http://{stage}@127.0.0.1:19000");
        let result = start_clients(endpoint.clone(), async {
            let proxy = if stage == "client slot" {
                Some(
                    ProxyHandle::start(
                        ProxyConfig::default(),
                        Arc::default(),
                        KeyCell::new(KeyState::Missing(KeyNeed::Unlock)),
                    )
                    .await
                    .unwrap(),
                )
            } else {
                None
            };
            if let Some(proxy) = proxy {
                proxy.shutdown().await;
            }
            Err::<(), _>(KeyError::Settings(format!("injected {stage} failure")))
        })
        .await;
        assert!(result.is_err());
        assert!(
            !clear_system_proxy_if(&endpoint),
            "failed startup left a global endpoint"
        );
    }

    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let old = KeyService::start(config(old_dir.path())).await.unwrap();
    let new = KeyService::start(config(new_dir.path())).await.unwrap();
    let endpoint = new.network_forwarder.proxy_url();
    old.shutdown().await;
    drop(old);
    assert!(
        clear_system_proxy_if(&endpoint),
        "old shutdown/drop cleared the newer owner"
    );
    new.shutdown().await;

    let service = KeyService::start(config(old_dir.path())).await.unwrap();
    let endpoint = service.network_forwarder.proxy_url();
    service.shutdown().await;
    assert!(
        !clear_system_proxy_if(&endpoint),
        "shutdown left its global endpoint"
    );
    let service = KeyService::start(config(new_dir.path())).await.unwrap();
    let endpoint = service.network_forwarder.proxy_url();
    drop(service);
    assert!(
        !clear_system_proxy_if(&endpoint),
        "drop left its global endpoint"
    );
}

#[tokio::test]
async fn missing_key_and_locked_saved_proxy_have_distinct_update_readiness() {
    let _serial = LIFETIME_TEST.lock().await;
    let fresh = tempfile::tempdir().unwrap();
    let service = KeyService::start(config(fresh.path())).await.unwrap();
    assert!(vw_mistral::network::update_route_ready());
    let endpoint = service.network_forwarder.proxy_url();
    assert!(clear_system_proxy_if(&endpoint));
    service.shutdown().await;

    let locked = tempfile::tempdir().unwrap();
    let vault = Vault::new(config(locked.path()).vault).unwrap();
    vault
        .create(&vw_vault::SecretString::new("test vault password"), false)
        .unwrap();
    let key = crate::network_key::rotate_mac_key(&vault, false).unwrap();
    let settings = vw_mistral::network::NetworkSettings::default()
        .with("HTTPS_PROXY", Some("http://proxy.test:8080".into()));
    let prepared = vw_mistral::network::prepare_settings(&settings).unwrap();
    let sealed = vw_mistral::network::seal_settings(&prepared, Some(&key)).unwrap();
    let file = locked.path().join("network/settings.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, sealed).unwrap();
    vault.lock();
    drop(vault);

    let service = KeyService::start_with_network(config(locked.path()), Some(file))
        .await
        .unwrap();
    assert!(service.network_settings_wait().is_some());
    assert!(!vw_mistral::network::update_route_ready());
    let endpoint = service.network_forwarder.proxy_url();
    assert!(
        clear_system_proxy_if(&endpoint),
        "locked vault had no relay"
    );
    service
        .unlock_with_method(
            vw_vault::SecretString::new("test vault password"),
            None,
            None,
        )
        .await
        .unwrap();
    assert!(service.network_settings_wait().is_none());
    assert!(vw_mistral::network::update_route_ready());
    service.shutdown().await;
}
