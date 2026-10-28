//! Key and vault access by window and Debug API origins over fake services.
//! PREDICT: the window can manage secrets, while Debug API management is denied and reads omit values.

mod common;

use std::sync::Arc;

use common::core::no_vibe_core_with as script_core_with;
use common::fake_mistral;
use common::vault::key_service_config;
use serde_json::{Value, json};
use vw_core_api::{
    Command, CommandResult, ErrorCode, Origin, VAULT_USER_ONLY, VAULT_USER_ONLY_MESSAGE,
};
use vw_gate::Confirmation;
use vw_keys::KeyService;
use vw_keys::existing::MemoryVibeKeyring;
use vw_vault::SecretString;

const KEY_UI: &str = "uikeyUUUUUUUUUUUUUUUUUUUUUUUUUU1";
const KEY_EXT: &str = "extkeyXXXXXXXXXXXXXXXXXXXXXXXwxyz";

fn keys(op: Value) -> Command {
    serde_json::from_value(json!({"type": "keys", "request": op})).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn key_and_vault_management_is_the_users_the_debug_api_only_reads() {
    let pass = format!("test-passphrase-{}", vw_vault::login::new_id());
    let fake = fake_mistral::start().await;
    let dir = common::processes::scratch("keys-origin");
    std::fs::create_dir_all(dir.join("vibe-home")).unwrap();
    std::fs::write(
        dir.join("vibe-home/.env"),
        "MISTRAL_API_KEY=envkeyEEEEEEEEEEEEEEEEEEEEEEEEEE\n",
    )
    .unwrap();
    let ring = Arc::new(MemoryVibeKeyring::default());
    let service = KeyService::start(key_service_config(&dir, &fake.base(), ring))
        .await
        .unwrap();
    let rig = script_core_with("keys-origin-core", &[], |c| c.keys = Some(service.clone())).await;
    let run = |origin: Origin, op: Value| {
        let core = rig.core.clone();
        async move { core.execute(origin, keys(op)).await }
    };

    // the app window sets up the vault and the key without any dialog.
    run(
        Origin::Ui,
        json!({"op": "createVault", "passphrase": pass.as_str()}),
    )
    .await
    .unwrap();
    run(Origin::Ui, json!({"op": "setupStoreKey", "apiKey": KEY_UI}))
        .await
        .unwrap();
    assert!(
        rig.confirmer.shown().is_empty(),
        "the app window is not asked"
    );

    // refused before anything runs; the declining confirmer is never consulted.
    rig.confirmer.then(Confirmation::Declined);
    let refused = run(
        Origin::DebugApi,
        json!({"op": "setupStoreKey", "apiKey": KEY_EXT}),
    )
    .await
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::Denied, "{refused:?}");
    assert_eq!(refused.server_code.as_deref(), Some(VAULT_USER_ONLY));
    assert_eq!(refused.message, VAULT_USER_ONLY_MESSAGE);
    run(Origin::DebugApi, json!({"op": "testKey"}))
        .await
        .unwrap();
    assert_eq!(
        fake.requests().last().unwrap().authorization.as_deref(),
        Some(format!("Bearer {KEY_UI}").as_str()),
        "the key in use is still the window's"
    );

    // reads ask nothing and carry no secret value.
    for op in [
        "readStatus",
        "listKeyCandidates",
        "testKey",
        "detectVibeKey",
    ] {
        let answer = run(Origin::DebugApi, json!({"op": op})).await.unwrap();
        let text = serde_json::to_string(&answer).unwrap();
        for secret in [
            KEY_UI,
            KEY_EXT,
            pass.as_str(),
            "envkeyEEEEEEEEEEEEEEEEEEEEEEEEEE",
        ] {
            assert!(!text.contains(secret), "{op} answered a secret value");
        }
    }

    // every management op from the Debug API is refused and changes nothing.
    let status = |rig_core: vw_core_api::Core| async move {
        let CommandResult::KeyStatus { status } = rig_core
            .execute(Origin::Ui, keys(json!({"op": "readStatus"})))
            .await
            .unwrap()
        else {
            panic!()
        };
        (
            status.vault_state,
            status.key_state,
            status.remembered_device,
        )
    };
    let before = status(rig.core.clone()).await;
    for op in [
        json!({"op": "createVault", "passphrase": pass.as_str()}),
        json!({"op": "unlockVault", "passphrase": pass.as_str()}),
        json!({"op": "unlockVault", "passphrase": pass.as_str()}),
        json!({"op": "lockVault"}),
        json!({"op": "useExistingKey", "itemId": "vibe-work-mistral-key"}),
        json!({"op": "importVibeKey", "source": "dotEnv"}),
        json!({"op": "removeVibeKeyOriginal", "source": "dotEnv"}),
    ] {
        let refused = run(Origin::DebugApi, op.clone()).await.unwrap_err();
        assert_eq!(
            refused.server_code.as_deref(),
            Some(VAULT_USER_ONLY),
            "{op}: {refused:?}"
        );
        assert_eq!(
            status(rig.core.clone()).await,
            before,
            "{op} changed nothing"
        );
    }
    assert!(
        dir.join("vibe-home/.env").exists(),
        "Vibe's own copy is still there"
    );

    // The window does the same acts.
    run(Origin::Ui, json!({"op": "lockVault"})).await.unwrap();
    assert_eq!(status(rig.core.clone()).await.0, "locked");
    run(
        Origin::Ui,
        json!({"op": "unlockVault", "passphrase": pass.as_str()}),
    )
    .await
    .unwrap();
    assert!(
        rig.confirmer.shown().is_empty(),
        "{:?}",
        rig.confirmer.shown()
    );
    let _ = SecretString::new("");
    service.shutdown().await;
}
