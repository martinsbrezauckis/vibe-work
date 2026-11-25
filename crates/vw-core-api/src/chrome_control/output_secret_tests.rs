//! Stored-secret exports through the gateway, including frozen reads; reads the actual chat-folder bytes.
//! Uses the offline core and helper fixture; a private runner can supply a headless Chromium renderer.
use super::{gateway, output_tests, read_output_tests, test_support};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vw_secrets::{SecretKind, marker, register};

async fn assert_frozen_refusal(core: &crate::core::Core, read: &Value) {
    let reason = gateway::call(&core.shared, "s", "chrome_execute", read.clone())
        .await
        .expect_err("a changed complete receipt must refuse before freezing");
    assert!(
        reason.contains("key-like") && reason.contains("Narrow the read"),
        "{reason}"
    );
    assert!(reason.contains("do not replay writes"), "{reason}");
}

async fn frozen_export_read(
    core: &crate::core::Core,
    key_page: &AtomicBool,
    read: &Value,
) -> Value {
    assert_frozen_refusal(core, read).await;
    // Keep the cached-export coverage using the same registered values outside key context.
    key_page.store(false, Ordering::SeqCst);
    let first = gateway::call(&core.shared, "s", "chrome_execute", read.clone())
        .await
        .unwrap();
    assert!(
        first["vwMcpResult"]["structuredContent"]["continuation"]["args"].is_object(),
        "the fixture must exceed the inline budget to exercise a frozen export"
    );
    // The direct export still reads the original key page; the cursor export reads the safe frozen body.
    key_page.store(true, Ordering::SeqCst);
    first
}

async fn exported_secrets(mode: &str) {
    let (core, folder) = read_output_tests::fixture(&format!("chrome-secret-export-{mode}")).await;
    core.shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
    let vault = format!("export-vault-{mode}-\"\\-value");
    let app = format!("export-app-{mode}-credential");
    let mut raw = json!({"status":"completed","action":"page.read","data":{
        "readId":"rendered-account-page","text":format!("Account keys: {vault} {app} {}", "page text ".repeat(3000)),
        "refs":[{"name":vault,"value":app}],"coverage":{"complete":true}}});
    if let Some(renderer) = std::env::var_os("VW_CHROME_EXPORT_RENDERER") {
        let rendered = std::process::Command::new("node")
            .arg(renderer)
            .args([&vault, &app, &"page text ".repeat(3000)])
            .output()
            .unwrap();
        assert!(
            rendered.status.success(),
            "render fixture: {}",
            String::from_utf8_lossy(&rendered.stderr)
        );
        raw = serde_json::from_slice(&rendered.stdout).unwrap();
        let text = raw["data"]["text"].as_str().unwrap();
        assert!(
            text.contains(&vault) && text.contains(&app),
            "fixture renders both registered secrets"
        );
    }
    let copy = raw.clone();
    let key_page = Arc::new(AtomicBool::new(true));
    let context = key_page.clone();
    let mut plain = raw.clone();
    plain["data"]["text"] = json!(format!(
        "Account details: {vault} {app} {}",
        "page text ".repeat(3000)
    ));
    let seen = test_support::answering(&core.shared, move |tool, args| {
        if tool == "chrome_status" {
            output_tests::status(args)
        } else {
            output_tests::mcp(if context.load(Ordering::SeqCst) {
                copy.clone()
            } else {
                plain.clone()
            })
        }
    });
    let read = json!({"sessionId":"t","action":"page.read","args":{}});
    let first = frozen_export_read(&core, &key_page, &read).await;
    register(SecretKind::VaultValue, &vault);
    register(SecretKind::ConnectorCredential, &app);
    // Registration after freezing must still protect exports; a fresh inline read now refuses too.
    key_page.store(false, Ordering::SeqCst);
    assert_frozen_refusal(&core, &read).await;
    key_page.store(true, Ordering::SeqCst);
    for args in [
        json!({}),
        first["vwMcpResult"]["structuredContent"]["continuation"]["args"].clone(),
    ] {
        let request = json!({"sessionId":"t","action":"page.read","args":args,"saveTo":"file"});
        let reply = gateway::call(&core.shared, "s", "chrome_execute", request)
            .await
            .unwrap();
        let file = &reply["vwMcpResult"]["structuredContent"]["savedToFile"];
        let bytes = std::fs::read(folder.join(file["path"].as_str().expect("saved file"))).unwrap();
        assert!(
            vw_secrets::find(&bytes).is_none(),
            "stored secret reached the saved FILE in {mode}"
        );
        let saved: Value = serde_json::from_slice(&bytes).expect("scrub preserves JSON");
        let text = saved["untrustedPageContent"]["data"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains(&marker(SecretKind::VaultValue)));
        assert!(text.contains(&marker(SecretKind::ConnectorCredential)));
        assert!(text.ends_with(&"page text ".repeat(3000)));
        assert_eq!(file["bytes"], bytes.len());
        assert_eq!(
            saved["untrustedPageContent"]["data"]["coverage"],
            raw["data"]["coverage"]
        );
    }
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "chrome_execute")
            .count(),
        4
    );
    core.shutdown().await;
}

#[tokio::test]
async fn stored_secrets_never_reach_exported_files_in_ask() {
    exported_secrets("ask").await;
}

#[tokio::test]
async fn stored_secrets_never_reach_exported_files_in_full() {
    exported_secrets("auto-approve").await;
}

#[tokio::test]
async fn stored_secrets_never_reach_exported_files_in_accept_edits() {
    exported_secrets("accept-edits").await;
}
