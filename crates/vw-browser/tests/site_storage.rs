//! Current-site storage actions through the browser driver on the fixed synthetic regression origin.
//! Uses the isolated Chromium fixture; native engine coverage lives in the private playground runner.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::Actor;

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

async fn act(rig: &Rig, args: Value) -> Value {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&args).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap()["untrustedPageContent"]
        .clone()
}

#[tokio::test]
async fn storage_actions_round_trip_both_areas_and_clear_only_the_selected_area() {
    let rig = Rig::new().await;
    for area in ["local", "session"] {
        let result = act(&rig, json!({"action":"storage.change","area":area,"operation":"set","key":"fixture","value":"hello"})).await;
        assert_eq!(result["value"], "hello");
        let result = act(&rig, json!({"action":"storage.metadata","area":area})).await;
        assert!(
            result["keys"]
                .as_array()
                .unwrap()
                .contains(&json!("fixture"))
        );
        let result = act(
            &rig,
            json!({"action":"storage.read","area":area,"keys":["fixture","missing"]}),
        )
        .await;
        assert_eq!(result["values"]["fixture"], "hello");
        assert_eq!(result["values"]["missing"], Value::Null);
        act(
            &rig,
            json!({"action":"storage.change","area":area,"operation":"remove","key":"fixture"}),
        )
        .await;
        assert_eq!(
            act(
                &rig,
                json!({"action":"storage.read","area":area,"keys":["fixture"]})
            )
            .await["values"]["fixture"],
            Value::Null
        );
    }
    for area in ["local", "session"] {
        act(&rig, json!({"action":"storage.change","area":area,"operation":"set","key":"keep","value":"yes"})).await;
    }
    act(
        &rig,
        json!({"action":"storage.change","area":"local","operation":"clear"}),
    )
    .await;
    assert_eq!(
        act(&rig, json!({"action":"storage.metadata","area":"local"})).await["keys"],
        json!([])
    );
    assert_eq!(
        act(
            &rig,
            json!({"action":"storage.read","area":"session","keys":["keep"]})
        )
        .await["values"]["keep"],
        "yes"
    );
}

#[test]
fn storage_arguments_reject_wrong_areas_operations_and_unbounded_values() {
    for args in [
        json!({"action":"storage.metadata","area":"other"}),
        json!({"action":"storage.read","area":"local"}),
        json!({"action":"storage.change","area":"local","operation":"execute","key":"x"}),
        json!({"action":"storage.change","area":"local","operation":"set","key":"x","value":"x".repeat(60_001)}),
    ] {
        assert!(vw_browser::parse(&args).is_err(), "{args}");
    }
}

#[tokio::test]
async fn storage_withholds_registered_vault_values_and_preserves_literal_keys() {
    const SECRET: &str = "fixture-vault-value-never-returned";
    let rig = Rig::new().await;
    vw_secrets::register(vw_secrets::SecretKind::VaultValue, SECRET);
    rig.engine
        .eval(&format!(
            "localStorage.setItem('__proto__',{});",
            json!(SECRET)
        ))
        .await;
    let result = act(
        &rig,
        json!({"action":"storage.read","area":"local","keys":["__proto__"]}),
    )
    .await;
    assert!(!result.to_string().contains(SECRET));
    assert!(result["values"]["__proto__"].is_string());
}
