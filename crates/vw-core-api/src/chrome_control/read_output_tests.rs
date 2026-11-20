//! Read-page budgets through the real gateway with a deterministic helper; exports live in `read_export_tests`.
//! No browser, model, shell or network; bytes include the app envelope and code-mode return indentation.
use super::{gateway, output_tests, test_support};
use serde_json::{Value, json};
use std::path::PathBuf;

pub(super) async fn fixture(label: &str) -> (crate::Core, PathBuf) {
    let core = output_tests::core(label).await;
    let folder = core
        .shared
        .instructions
        .own_home
        .parent()
        .unwrap()
        .join("chat");
    std::fs::create_dir_all(&folder).unwrap();
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"s","cwd":folder},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    (core, folder)
}

pub(super) fn request(args: Value) -> Value {
    let mut request = json!({"sessionId":"t","action":"page.read"});
    request["args"] = args;
    request
}

#[tokio::test]
async fn reads_send_a_useful_bounded_budget_and_leave_cursor_arguments_exact() {
    let (core, folder) = fixture("chrome-focused-default").await;
    let seen = test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            return output_tests::status(args);
        }
        output_tests::mcp(
            json!({"status":"completed","action":"page.read","data":{"readId":"r","text":"Small read"}}),
        )
    });
    for args in [
        json!({}),
        json!({"maxChars":4000}),
        json!({"find":"Email","frameId":2}),
    ] {
        gateway::call(&core.shared, "s", "chrome_execute", request(args))
            .await
            .unwrap();
    }
    gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        request(json!({"cursor":"h-opaque/😀+=\\","tabId":7})),
    )
    .await
    .unwrap();
    let calls = seen.lock().unwrap().clone();
    let reads: Vec<_> = calls
        .iter()
        .filter(|(tool, _)| tool == "chrome_execute")
        .collect();
    assert_eq!(reads.len(), 4);
    for (_, args) in &reads[..3] {
        assert_eq!(args["args"]["maxBytes"], 10_000, "{args}");
        assert_eq!(args["args"]["limit"], 32, "{args}");
    }
    assert_eq!(
        reads[3].1["args"],
        json!({"cursor":"h-opaque/😀+=\\","tabId":7})
    );
    assert_eq!(std::fs::read_dir(folder).unwrap().count(), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn four_explicitly_small_read_pages_fit_one_code_mode_return() {
    let (core, folder) = fixture("chrome-four-read-pages").await;
    core.shared.chrome_control.owner.stop().await;
    let seen = test_support::answering(&core.shared, |_, args| {
        let budget = args["args"]["maxBytes"].as_u64().unwrap_or(3000);
        let count = (budget.saturating_sub(600) / 140)
            .min(args["args"]["limit"].as_u64().unwrap_or(32)) as usize;
        let refs: Vec<_> = (0..count).map(|i| json!({"ref":format!("e{i:08x}"),"role":"button","tag":"button",
            "name":format!("Contact person {i} in the directory"),"disabled":false,"required":false,"frameId":0})).collect();
        output_tests::mcp(
            json!({"operationId":"read","sessionId":"t","status":"completed","action":"page.read",
            "data":{"readId":"capture","tabId":7,"documentId":"document-a","textPreview":"Directory contacts. ".repeat(16),"refs":refs,
                "coverage":{"complete":false},"continuation":{"refs":{"action":"page.read","args":{"tabId":7,"cursor":"h-next"},"reason":"remaining_refs"}}}}),
        )
    });
    let mut reads = Vec::new();
    for _ in 0..4 {
        let reply = gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            request(json!({"maxBytes":2000,"limit":8,"maxChars":800})),
        )
        .await
        .unwrap();
        let body = reply["vwMcpResult"]["structuredContent"].clone();
        println!(
            "READ bytes: compact={}, pretty={}",
            body.to_string().len(),
            serde_json::to_string_pretty(&body).unwrap().len()
        );
        reads.push(body);
    }
    let returned = serde_json::to_string_pretty(&json!({"result":{"reads":reads}})).unwrap();
    println!(
        "FOUR READS: bytes={}, spills-at-16000={}",
        returned.len(),
        usize::from(returned.len() > 16_000)
    );
    assert!(
        returned.len() <= 16_000,
        "combined read pages spill: {}",
        returned.len()
    );
    assert_eq!(std::fs::read_dir(folder).unwrap().count(), 0);
    assert_eq!(seen.lock().unwrap().len(), 4);
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(tool, _)| tool == "chrome_execute"),
        "unexpected helper calls: {:?}",
        seen.lock().unwrap()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn long_article_lists_keep_useful_text_and_thirty_two_links_in_one_read() {
    let core = output_tests::core("browser-long-article-list").await;
    let text = "Article title and useful summary. ".repeat(180);
    let expected_text = text.clone();
    test_support::answering(&core.shared, move |tool, args| {
        if tool == "chrome_status" {
            return output_tests::status(args);
        }
        let limit = args["args"]["limit"].as_u64().unwrap_or(200).min(200);
        let refs: Vec<_> = (0..limit).map(|i| json!({"ref":format!("e{i}"),"role":"link","name":format!("Article {i}"),"href":format!("https://example.test/{i}")})).collect();
        let max_bytes =
            usize::try_from(args["args"]["maxBytes"].as_u64().unwrap_or(64000)).unwrap();
        let max_chars = usize::try_from(args["args"]["maxChars"].as_u64().unwrap_or(800)).unwrap();
        output_tests::mcp(
            json!({"operationId":"read-articles","sessionId":"t","status":"completed","action":"page.read",
            "data":{"readId":"article-list","documentId":"document-a","tabId":7,"text":&text[..text.len().min(max_bytes).min(max_chars)],"refs":refs}}),
        )
    });
    let answer = gateway::call(&core.shared, "s", "chrome_execute", request(json!({})))
        .await
        .unwrap();
    let body = &answer["vwMcpResult"]["structuredContent"];
    let data = &body["untrustedPageContent"]["data"];
    assert_eq!(
        data["text"], expected_text,
        "a useful article list should not require many tiny reads: {body}"
    );
    assert_eq!(data["refs"].as_array().map(Vec::len), Some(32), "{body}");
    assert!(body.to_string().len() <= 12_000);
    assert!(serde_json::to_string_pretty(body).unwrap().len() <= 15_000);
    core.shutdown().await;
}

#[test]
fn read_code_mode_wrapper_budget_counts_its_indentation() {
    let mut oversized = Vec::new();
    for count in [32, 64, 96, 128, 160, 200] {
        for text_len in [2000, 4000, 6000, 8000, 10000] {
            let refs: Vec<_> = (0..count)
                .map(|i| json!({"ref":format!("e{i}"),"role":"link"}))
                .collect();
            let raw = output_tests::mcp(json!({"action":"page.read","status":"completed",
                "data":{"documentId":"article","text":"t".repeat(text_len),"refs":refs}}));
            let result = super::output::for_model(
                super::owner::Answer::Helper(raw),
                "chrome_execute",
                false,
            );
            let body = &result["vwMcpResult"]["structuredContent"];
            let wrapped =
                serde_json::to_string_pretty(&json!({"result":{"reads":[body]}})).unwrap();
            if wrapped.len() > 16_000 {
                oversized.push((count, text_len, wrapped.len()));
            }
        }
    }
    assert!(
        oversized.is_empty(),
        "full-size single reads must fit the real code-mode wrapper: {oversized:?}"
    );
}
