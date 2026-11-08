//! Final app budgets for control-first reads, chooser receipts and uploads, including older helper receipts.
//! Exercises page marking and production output compaction without a browser or window.
use super::{content, output, output_tests::mcp};
use serde_json::{Value, json};

fn finish(action: &str, data: &Value) -> Value {
    let mut result = mcp(json!({
        "operationId":"page-operation", "sessionId":"browser-session", "status":"completed",
        "action":action, "data":data
    }));
    content::mark(&mut result);
    output::finish(result, "chrome_execute", false)["vwMcpResult"].take()
}

#[test]
fn an_upload_never_exposes_the_editors_value_from_an_older_helper() {
    let result = finish(
        "files.upload",
        &json!({"fileId":"selected", "fileIds":["selected"], "files":[{"name":"dispatch.txt", "size":4}],
            "fieldReplaced":true, "value":"Text from the unrelated message editor", "route":"DataTransfer"}),
    );
    let data = &result["structuredContent"]["untrustedPageContent"]["data"];
    assert!(data.get("value").is_none(), "{result}");
    assert_eq!(data["files"][0]["name"], "dispatch.txt");
    assert_eq!(data["fileIds"], json!(["selected"]));
    assert!(!result.to_string().contains("unrelated message editor"));
}

#[test]
fn a_chooser_instruction_survives_the_final_app_budget() {
    let result = finish(
        "page.click",
        &json!({"effect":"file_chooser", "chooser":{"mode":"selectSingle"},
            "note":"File chooser opened — use files.upload.", "title":"Long page title. ".repeat(2000)}),
    );
    let body = &result["structuredContent"];
    assert!(output::fits(body, 4000));
    let data = &body["untrustedPageContent"]["data"];
    assert_eq!(data["effect"], "file_chooser");
    assert_eq!(data["note"], "File chooser opened — use files.upload.");
    assert_eq!(body["status"], "completed");
}

#[test]
fn control_first_read_pages_keep_refs_and_the_exact_continuation_under_four_kib() {
    let names = ["To recipients", "Subject", "Category", "Message Body"];
    for (page, chunk) in names.chunks(3).enumerate() {
        let rows: Vec<_> = chunk
            .iter()
            .enumerate()
            .map(|(i, name)| {
                json!({"ref":format!("field-{page}-{i}"), "tabId":7, "frameId":0,
                "documentId":"compose-document", "origin":"https://example.test",
                "tag":"input", "role":"textbox", "name":name, "value":""})
            })
            .collect();
        let next = json!({"action":"page.read", "args":{"tabId":7,"cursor":"opaque:refs:3:0"}});
        let result = finish(
            "page.read",
            &json!({"readId":"capture", "documentId":"compose-document",
            "refs":rows, "textPreview":"Dispatch archive", "continuation":{"refs":next}}),
        );
        let body = &result["structuredContent"];
        assert!(output::fits(body, 4000));
        let data = &body["untrustedPageContent"]["data"];
        assert_eq!(data["refs"], json!(rows));
        assert_eq!(data["continuation"]["refs"], next);
        assert_eq!(body["status"], "completed");
    }
}
