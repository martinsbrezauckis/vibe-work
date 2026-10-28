//! The vault keeps its last state across restarts: an unlocked vault opens unlocked at the next start (its key is kept
//! in the computer's credential store), Lock deletes the kept key, and the "Lock the vault when Vibe Work quits"
//! switch deletes it at quit and, after an abrupt end, at the next start. Installs from before the switch get it from
//! their old "remember this computer" choice, once.
//! A restart is simulated by a second key service on the same folder and the same credential store.
//! PREDICT: every state survives the restart as described; the Debug API reads the switch and may not change it.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::core::no_vibe_core_with as script_core_with;
use common::fake_mistral;
use common::vault::{client, key_service_config, memory_store, passphrase, profile_json};
use serde_json::{Value, json};
use vw_core_api::{Command, CommandResult, ErrorCode, Origin, VAULT_USER_ONLY};
use vw_keys::KeyService;
use vw_keys::existing::MemoryVibeKeyring;
use vw_vault::VaultStatus;
use vw_vault::remember::{DeviceSecretStore, account_for};

/// The file the switch lives in, next to the key settings.
fn vault_settings(dir: &Path) -> std::path::PathBuf {
    dir.join("vault-settings.json")
}

/// The kept device key of the test's profile folder, if the credential store has one.
fn kept_key(dir: &Path, store: &Arc<dyn DeviceSecretStore>) -> bool {
    store
        .get(&account_for(&dir.join("shellx-vault")))
        .unwrap()
        .is_some()
}

/// One start of the key service on `dir` with `store` as the credential store.
async fn start(dir: &Path, base: &str, store: &Arc<dyn DeviceSecretStore>) -> Arc<KeyService> {
    let mut config = key_service_config(dir, base, Arc::new(MemoryVibeKeyring::default()));
    config.vault.device_store = Arc::clone(store);
    KeyService::start(config).await.unwrap()
}

async fn unlocked(service: &KeyService) -> bool {
    matches!(
        service.status().await.unwrap().vault,
        VaultStatus::Unlocked { .. }
    )
}

#[test]
fn lock_deletes_the_kept_key_and_close_keeps_it() {
    let dir = common::vault::profile_dir("last-state-lock");
    let store = memory_store();
    let vault = client(&dir, Arc::clone(&store));
    vault.create(&passphrase(), true).unwrap();
    vault.close();
    assert!(
        client(&dir, Arc::clone(&store))
            .unlock_remembered()
            .unwrap(),
        "closing (a quit) keeps the key: the next client opens unlocked"
    );
    vault.unlock(&passphrase(), None, None).unwrap();
    vault.lock();
    assert!(
        store.get(&account_for(&dir)).unwrap().is_none(),
        "Lock deletes the kept key from the credential store"
    );
    assert_eq!(profile_json(&dir)["rememberDevice"], false);
    assert!(
        profile_json(&dir)
            .get("rememberedKeyfileJson")
            .is_none_or(Value::is_null)
    );
    assert!(
        !client(&dir, store).unlock_remembered().unwrap(),
        "after Lock the next start stays locked"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unlocked_stays_unlocked_and_lock_stays_locked_across_restarts() {
    let fake = fake_mistral::start().await;
    let dir = common::processes::scratch("last-state-restart");
    let store = memory_store();

    let first = start(&dir, &fake.base(), &store).await;
    assert!(
        !first.lock_on_quit(),
        "a new install keeps the vault's last state"
    );
    first.create_vault(passphrase()).await.unwrap();
    assert!(
        kept_key(&dir, &store),
        "unlocking keeps the key on this computer"
    );
    first.shutdown().await;
    assert!(
        kept_key(&dir, &store),
        "a quit with the switch off keeps the key"
    );

    let second = start(&dir, &fake.base(), &store).await;
    assert!(
        unlocked(&second).await,
        "unlocked at quit: unlocked at the next start"
    );
    second.lock();
    assert!(!kept_key(&dir, &store), "Lock deletes the kept key");
    second.shutdown().await;

    let third = start(&dir, &fake.base(), &store).await;
    assert!(
        !unlocked(&third).await,
        "locked at quit: locked at the next start"
    );
    third.unlock(passphrase(), None).await.unwrap();
    assert!(
        kept_key(&dir, &store),
        "a password unlock keeps the key again"
    );
    third.shutdown().await;
    let fourth = start(&dir, &fake.base(), &store).await;
    assert!(unlocked(&fourth).await);
    fourth.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_switch_locks_at_quit_and_after_an_abrupt_end() {
    let fake = fake_mistral::start().await;
    let dir = common::processes::scratch("last-state-switch");
    let store = memory_store();

    let first = start(&dir, &fake.base(), &store).await;
    first.create_vault(passphrase()).await.unwrap();
    first.set_lock_on_quit(true).unwrap();
    assert!(
        unlocked(&first).await,
        "turning the switch on does not lock now"
    );
    first.shutdown().await;
    assert!(
        !kept_key(&dir, &store),
        "quitting with the switch on deletes the key"
    );
    let second = start(&dir, &fake.base(), &store).await;
    assert!(second.lock_on_quit(), "the switch is kept");
    assert!(!unlocked(&second).await, "and the vault starts locked");

    // An abrupt end: unlocked with the switch on, and no shutdown runs (crash, power off, killed process).
    second.unlock(passphrase(), None).await.unwrap();
    assert!(kept_key(&dir, &store), "the key is kept while the app runs");
    let third = start(&dir, &fake.base(), &store).await;
    assert!(
        !unlocked(&third).await,
        "the next start does not use the key"
    );
    assert!(!kept_key(&dir, &store), "it deletes it instead");
    third.shutdown().await;
    drop(second);

    // The same abrupt end with the switch off is a normal quit: the vault opens unlocked.
    let fourth = start(&dir, &fake.base(), &store).await;
    fourth.set_lock_on_quit(false).unwrap();
    fourth.unlock(passphrase(), None).await.unwrap();
    let fifth = start(&dir, &fake.base(), &store).await;
    assert!(
        unlocked(&fifth).await,
        "switch off: an abrupt end keeps the last state"
    );
    fifth.shutdown().await;
    drop(fourth);
}

/// An install from before the switch: a vault made with "remember this computer" `remember`, and the old key
/// settings file (only `keyItemId`), no vault settings file.
fn old_install(dir: &Path, store: &Arc<dyn DeviceSecretStore>, remember: bool) {
    let vault = client(&dir.join("shellx-vault"), Arc::clone(store));
    vault.create(&passphrase(), remember).unwrap();
    vault.close();
    std::fs::write(
        dir.join("keys.json"),
        r#"{"keyItemId":"vibe-work-mistral-api-key"}"#,
    )
    .unwrap();
    assert!(!vault_settings(dir).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn migration_turns_the_switch_on_only_for_installs_that_did_not_remember() {
    let fake = fake_mistral::start().await;

    // "Remember this computer" was off: the switch comes on, nothing changes for this user.
    let dir = common::processes::scratch("last-state-migrate-off");
    let store = memory_store();
    old_install(&dir, &store, false);
    let first = start(&dir, &fake.base(), &store).await;
    assert!(first.lock_on_quit());
    assert!(!unlocked(&first).await, "they still unlock at start");
    let written = std::fs::read(vault_settings(&dir)).unwrap();
    let settings: Value = serde_json::from_slice(&written).unwrap();
    assert_eq!(settings["lockVaultOnQuit"], true);
    first.shutdown().await;
    // Once: a later start reads the switch and leaves the file as it is.
    let again = start(&dir, &fake.base(), &store).await;
    assert!(again.lock_on_quit());
    assert_eq!(std::fs::read(vault_settings(&dir)).unwrap(), written);
    // The user's later choice wins over the old flag (no second migration).
    again.set_lock_on_quit(false).unwrap();
    again.unlock(passphrase(), None).await.unwrap();
    again.shutdown().await;
    let later = start(&dir, &fake.base(), &store).await;
    assert!(!later.lock_on_quit());
    assert!(unlocked(&later).await);
    later.shutdown().await;

    // "Remember this computer" was on: the switch stays off (the update's first start is below).
    let dir = common::processes::scratch("last-state-migrate-on");
    let store = memory_store();
    old_install(&dir, &store, true);
    let first = start(&dir, &fake.base(), &store).await;
    assert!(!first.lock_on_quit());
    first.shutdown().await;
    let settings: Value =
        serde_json::from_slice(&std::fs::read(vault_settings(&dir)).unwrap()).unwrap();
    assert_eq!(settings["lockVaultOnQuit"], false);
    let again = start(&dir, &fake.base(), &store).await;
    assert!(!again.lock_on_quit());
    again.shutdown().await;
}

/// The files and the credential store entry an install from before the switch leaves at quit.
fn old_install_files(dir: &Path, store: &Arc<dyn DeviceSecretStore>) -> (Value, String, bool) {
    (
        profile_json(&dir.join("shellx-vault")),
        std::fs::read_to_string(dir.join("keys.json")).unwrap(),
        kept_key(dir, store),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_from_a_remembered_install_starts_locked_whether_or_not_it_was_locked() {
    let fake = fake_mistral::start().await;
    let mut left_behind = Vec::new();
    for (name, locked_before_update) in [("unlocked", false), ("locked", true)] {
        let dir = common::processes::scratch(&format!("last-state-update-{name}"));
        let store = memory_store();
        old_install(&dir, &store, true);
        if locked_before_update {
            // Lock before the switch existed only dropped the open vault in memory, as a quit does: it wrote no
            // file and kept the credential store entry.
            let vault = client(&dir.join("shellx-vault"), Arc::clone(&store));
            vault.unlock(&passphrase(), None, None).unwrap();
            vault.close();
        }
        left_behind.push(old_install_files(&dir, &store));

        let first = start(&dir, &fake.base(), &store).await;
        assert!(
            !unlocked(&first).await,
            "{name} before the update: the first start after it is locked"
        );
        assert!(!first.lock_on_quit(), "{name}: the switch is off");
        assert!(kept_key(&dir, &store), "{name}: the kept key stays");
        first.unlock(passphrase(), None).await.unwrap();
        first.shutdown().await;

        let second = start(&dir, &fake.base(), &store).await;
        assert!(
            unlocked(&second).await,
            "{name}: from then on the last state counts (unlocked at quit, unlocked now)"
        );
        second.lock();
        second.shutdown().await;
        let third = start(&dir, &fake.base(), &store).await;
        assert!(
            !unlocked(&third).await,
            "{name}: locked at quit, locked now"
        );
        third.shutdown().await;
    }
    // Nothing on disk or in the credential store tells the two apart (the profile's random parts aside).
    let shape = |(profile, keys, kept): &(Value, String, bool)| {
        let mut fields: Vec<_> = profile.as_object().unwrap().keys().cloned().collect();
        fields.sort();
        (
            fields,
            profile["rememberDevice"].clone(),
            keys.clone(),
            *kept,
        )
    };
    assert_eq!(shape(&left_behind[0]), shape(&left_behind[1]));
}

#[tokio::test(flavor = "multi_thread")]
async fn after_the_update_only_the_password_opens_the_vault_the_first_time() {
    // The first start after the update ends without an unlock: the vault was locked at that quit, so it stays
    // locked at the next start, and the kept key does not open it for an unlock action either.
    let fake = fake_mistral::start().await;
    let dir = common::processes::scratch("last-state-update-no-unlock");
    let store = memory_store();
    old_install(&dir, &store, true);
    let first = start(&dir, &fake.base(), &store).await;
    assert!(!unlocked(&first).await);
    first.shutdown().await;
    let second = start(&dir, &fake.base(), &store).await;
    assert!(
        !unlocked(&second).await,
        "locked at quit: locked at the next start"
    );
    assert!(
        !second.unlock_remembered().await.unwrap(),
        "the kept key opens nothing before the password"
    );
    assert!(kept_key(&dir, &store), "the kept key stays");
    second.unlock(passphrase(), None).await.unwrap();
    let settings: Value =
        serde_json::from_slice(&std::fs::read(vault_settings(&dir)).unwrap()).unwrap();
    assert_eq!(
        settings.get("lockedUntilPassword"),
        None,
        "the password unlock ends the wait"
    );
    second.shutdown().await;
    let third = start(&dir, &fake.base(), &store).await;
    assert!(unlocked(&third).await, "unlocked at quit: unlocked now");
    third.shutdown().await;
}

fn keys(op: Value) -> Command {
    serde_json::from_value(json!({"type": "keys", "request": op})).unwrap()
}

async fn status_of(core: &vw_core_api::Core, origin: Origin) -> Value {
    let CommandResult::KeyStatus { status } = core
        .execute(origin, keys(json!({"op": "readStatus"})))
        .await
        .unwrap()
    else {
        panic!("not a status")
    };
    serde_json::to_value(status).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_debug_api_reads_the_switch_and_only_the_window_changes_it() {
    let fake = fake_mistral::start().await;
    let dir = common::processes::scratch("last-state-debug-api");
    let store = memory_store();
    let service = start(&dir, &fake.base(), &store).await;
    let rig = script_core_with("last-state-debug-api-core", &[], |c| {
        c.keys = Some(service.clone());
    })
    .await;
    rig.core
        .execute(
            Origin::Ui,
            keys(json!({"op": "createVault", "passphrase": "last state test passphrase"})),
        )
        .await
        .unwrap();
    assert_eq!(
        status_of(&rig.core, Origin::DebugApi).await["lockVaultOnQuit"],
        false
    );

    let refused = rig
        .core
        .execute(
            Origin::DebugApi,
            keys(json!({"op": "setLockVaultOnQuit", "on": true})),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::Denied, "{refused:?}");
    assert_eq!(refused.server_code.as_deref(), Some(VAULT_USER_ONLY));
    assert!(!service.lock_on_quit(), "a refused change changes nothing");

    rig.core
        .execute(
            Origin::Ui,
            keys(json!({"op": "setLockVaultOnQuit", "on": true})),
        )
        .await
        .unwrap();
    assert!(service.lock_on_quit());
    assert_eq!(
        status_of(&rig.core, Origin::DebugApi).await["lockVaultOnQuit"],
        true
    );
    // The old "remember this computer" field is gone from the commands.
    assert!(
        serde_json::from_value::<Command>(json!({"type": "keys", "request":
            {"op": "unlockVault", "passphrase": "x", "rememberDevice": false}}))
        .is_err()
    );
    rig.core.shutdown().await;
    service.shutdown().await;
}

/// The app's own layout: a private vault next to the app data, and a shared ShellX vault that may exist.
async fn start_managed(
    dir: &Path,
    base: &str,
    store: &Arc<dyn DeviceSecretStore>,
) -> Arc<KeyService> {
    let mut config = key_service_config(dir, base, Arc::new(MemoryVibeKeyring::default()));
    config.vault.device_store = Arc::clone(store);
    config.managed_vault = Some(dir.join("app-data"));
    KeyService::start(config).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_private_vault_keeps_its_state_and_a_found_vault_is_no_old_choice() {
    let fake = fake_mistral::start().await;
    let dir = common::processes::scratch("last-state-managed");
    let store = memory_store();
    // A shared ShellX vault made without "remember this computer", never chosen by this install.
    let shared = client(&dir.join("shellx-vault"), Arc::clone(&store));
    shared.create(&passphrase(), false).unwrap();
    shared.close();

    let first = start_managed(&dir, &fake.base(), &store).await;
    assert!(
        !first.lock_on_quit(),
        "a vault found but not chosen is not this install's old choice"
    );
    first.create_vault(passphrase()).await.unwrap();
    first.shutdown().await;

    let second = start_managed(&dir, &fake.base(), &store).await;
    assert!(
        unlocked(&second).await,
        "the private vault opens unlocked after a restart"
    );
    second.lock();
    second.shutdown().await;
    let third = start_managed(&dir, &fake.base(), &store).await;
    assert!(!unlocked(&third).await, "and stays locked after Lock");
    third.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kept_key_the_credential_store_lost_is_not_reported_as_kept() {
    let fake = fake_mistral::start().await;
    let dir = common::processes::scratch("last-state-store-lost");
    let first = start(&dir, &fake.base(), &memory_store()).await;
    first.create_vault(passphrase()).await.unwrap();
    first.shutdown().await;
    // The next start has an empty credential store (no Secret Service, or the entry was deleted by hand).
    let second = start(&dir, &fake.base(), &memory_store()).await;
    match second.status().await.unwrap().vault {
        VaultStatus::Locked {
            remembered_device, ..
        } => assert!(!remembered_device, "locked, and no key is kept"),
        other => panic!("expected locked, got {other:?}"),
    }
    assert_eq!(
        profile_json(&dir.join("shellx-vault"))["rememberDevice"],
        false
    );
    second.shutdown().await;
}
