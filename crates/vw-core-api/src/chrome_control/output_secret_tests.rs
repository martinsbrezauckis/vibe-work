//! Stored-secret exports through the gateway, including frozen reads; reads the actual chat-folder bytes.
//! Uses the offline core and helper fixture; a private runner can supply a headless Chromium renderer.
use super::{gateway, output_tests, read_output_tests, test_support};
use serde_json::{Value, json};
use vw_secrets::{SecretKind, marker, register};

async fn exported_secrets(mode: &str) {
    let (core, folder) = read_output_tests::fixture(&format!("chrome-secret-export-{mode}")).await;
    core.shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
    let vault = format!("export-vault-{mode}-\"\\-value");
    let app = format!("export-app-{mode}-credential");
    register(SecretKind::VaultValue, &vault);
    register(SecretKind::ConnectorCredential, &app);
    let mut raw = json!({"status":"completed","action":"page.read","data":{
        "readId":"rendered-account-page","text":format!("Account keys: {vault} {app} {}", "page text ".repeat(1000)),
        "refs":[{"name":vault,"value":app}],"coverage":{"complete":true}}});
    if let Some(renderer) = std::env::var_os("VW_CHROME_EXPORT_RENDERER") {
        let rendered = std::process::Command::new("node")
            .arg(renderer)
            .args([&vault, &app, &"page text ".repeat(1000)])
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
    let seen = test_support::answering(&core.shared, move |tool, args| {
        if tool == "chrome_status" {
            output_tests::status(args)
        } else {
            output_tests::mcp(copy.clone())
        }
    });
    let read = json!({"sessionId":"t","action":"page.read","args":{}});
    let first = gateway::call(&core.shared, "s", "chrome_execute", read.clone())
        .await
        .unwrap();
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
        assert!(text.ends_with(&"page text ".repeat(1000)));
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
        2
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
