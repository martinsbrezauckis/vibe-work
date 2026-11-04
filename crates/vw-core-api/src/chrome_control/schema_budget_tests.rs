//! One code-mode return containing independent discovery calls, with Vibe's indent-two serialization.
//! Exercises the actual gateway, negotiated schemas and complete output envelope without a browser process.
use super::{capabilities, gateway, output_tests, test_support};
use serde_json::{Value, json};

fn status(args: &Value, modern: bool) -> Value {
    let mut result = output_tests::status(args);
    for profile in result["structuredContent"]["profiles"]
        .as_array_mut()
        .unwrap()
    {
        profile["extensionCapabilities"] = if modern {
            json!([
                "page.read.includeText",
                "page.read.cursor",
                "page.read.scope.viewport",
                "page.read.includeBounds",
                "page.observation.changes",
                "page.key.frameId",
                "page.read.summary",
                "page.find.scope.viewport"
            ])
        } else {
            json!([])
        };
    }
    result
}

#[tokio::test]
async fn twelve_single_action_lookups_fit_one_code_mode_program() {
    for modern in [false, true] {
        let core = output_tests::core("chrome-output-program").await;
        test_support::answering(&core.shared, move |_, args| status(args, modern));
        let mut results = Vec::new();
        for action in [
            "page.navigate",
            "page.read",
            "page.type",
            "page.select",
            "page.check",
            "page.uncheck",
            "page.click",
            "files.upload",
            "page.key",
            "page.find",
            "page.commit",
            "tabs.list",
        ] {
            let args = json!({"action":action});
            let mut expected = status(&args, modern);
            capabilities::filter(&mut expected, None);
            let answer = gateway::call(&core.shared, "s", "chrome_status", args)
                .await
                .unwrap();
            assert_eq!(answer["vwMcpResult"]["isError"], false);
            let body = answer["vwMcpResult"]["structuredContent"].clone();
            let row = &body["untrustedPageContent"];
            assert_eq!(row["action"], action);
            assert_eq!(row["supported"], true);
            assert_eq!(row["allowedInThisMode"], true);
            let source = &expected["structuredContent"]["capabilities"]["actions"][0];
            assert_eq!(output_tests::schema(row), source["inputSchema"]);
            assert_eq!(row["description"], source["description"]);
            assert!(body["note"].is_string());
            assert!(body["promptGuard"].is_object());
            results.push(body);
        }
        core.shutdown().await;
        // Equivalent to one run_typescript program returning all its awaited chrome_status({action})
        // results, not nine programs or one chrome_status({actions}) call. Include the return envelope.
        for count in 9..=12 {
            let text =
                serde_json::to_string_pretty(&json!({"result":{"schemas": &results[..count]}}))
                    .unwrap();
            println!(
                "SIZE {count} separate lookups, modern={modern}: {} chars",
                text.chars().count()
            );
            assert!(
                text.chars().count() < 16_000,
                "{count} lookups spill: {}",
                text.chars().count()
            );
        }
    }
}
