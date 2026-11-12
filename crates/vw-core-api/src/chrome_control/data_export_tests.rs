//! Structured exports through the connected-browser gateway, using a deterministic captured read.
//! Exercises actual chat-folder files, continuation collections and app-owned arguments without an extension.
use super::{gateway, output_tests, read_output_tests, test_support};
use serde_json::json;

#[tokio::test]
async fn csv_exports_real_rows_and_keeps_export_options_out_of_the_helper() {
    let (core, folder) = read_output_tests::fixture("browser-data-csv").await;
    let seen = test_support::answering(&core.shared, |_, _| {
        output_tests::mcp(json!({
            "status":"completed","action":"page.read","data":{"url":"https://example.com/table","coverage":{"complete":true},
            "structure":[{"tables":[{"rows":[["Name","Price"],["Tea, green","=1+2"]]}]}]}
        }))
    });
    let request = json!({"sessionId":"t","action":"page.read","args":{"interactiveOnly":false,"ref":"table"},"saveTo":"csv","extract":{"kind":"table"}});
    let answer = gateway::call(&core.shared, "s", "chrome_execute", request)
        .await
        .unwrap();
    let body = &answer["vwMcpResult"]["structuredContent"];
    let path = body["savedToFile"]["path"]
        .as_str()
        .expect("CSV file receipt");
    let csv = std::fs::read_to_string(folder.join(path)).unwrap();
    assert_eq!(csv, "\"Name\",\"Price\"\r\n\"Tea, green\",\"'=1+2\"\r\n");
    assert_eq!(body["savedToFile"]["rows"], 1);
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(_, args)| args.get("extract").is_none() && args.get("saveTo").is_none())
    );
    core.shutdown().await;
}

#[tokio::test]
async fn paginated_collections_are_immutable_and_xlsx_cells_are_literal_text() {
    use std::io::Read;
    let (core, folder) = read_output_tests::fixture("browser-data-pages").await;
    let page = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let current = page.clone();
    test_support::answering(&core.shared, move |_, _| {
        output_tests::mcp(json!({"status":"completed","action":"page.read","data":{
        "url":format!("https://example.com/page/{}",current.load(std::sync::atomic::Ordering::SeqCst)),"coverage":{"complete":true},
        "structure":[{"lists":[{"items":["Common",if current.load(std::sync::atomic::Ordering::SeqCst)==0 {"First"} else {"=HYPERLINK(\"bad\")"}]}]}]}}))
    });
    let mut request = json!({"sessionId":"t","action":"page.read","args":{"interactiveOnly":false},"saveTo":"csv","extract":{"kind":"list"}});
    let first = gateway::call(&core.shared, "s", "chrome_execute", request.clone())
        .await
        .unwrap();
    let first = &first["vwMcpResult"]["structuredContent"]["savedToFile"];
    let original = std::fs::read(folder.join(first["path"].as_str().unwrap())).unwrap();
    page.store(1, std::sync::atomic::Ordering::SeqCst);
    request["appendFrom"] = first["collection"].clone();
    request["deduplicate"] = json!(true);
    request["saveTo"] = json!("xlsx");
    let second = gateway::call(&core.shared, "s", "chrome_execute", request)
        .await
        .unwrap();
    let second = &second["vwMcpResult"]["structuredContent"]["savedToFile"];
    assert_eq!(second["rows"], 3);
    assert_eq!(second["addedRows"], 1);
    assert_eq!(second["pages"], 2);
    assert_eq!(
        std::fs::read(folder.join(first["path"].as_str().unwrap())).unwrap(),
        original
    );
    let mut archive = zip::ZipArchive::new(
        std::fs::File::open(folder.join(second["path"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    let mut sheet = String::new();
    archive
        .by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut sheet)
        .unwrap();
    assert!(sheet.contains("=HYPERLINK(&quot;bad&quot;)"));
    assert!(!sheet.contains("<f>"));
    assert_eq!(sheet.matches("<row>").count(), 4);
    let bytes = std::fs::read(folder.join(second["path"].as_str().unwrap())).unwrap();
    let text = vw_documents::read_local(vw_documents::DocKind::Xlsx, &bytes).unwrap();
    assert!(
        text.contains("Common") && text.contains("First") && text.contains("HYPERLINK"),
        "{text}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn exports_scrub_stored_values_and_reject_incompatible_appends() {
    let (core, folder) = read_output_tests::fixture("browser-data-secrets").await;
    let secret = "StoredExportValueOnlyFixture123";
    vw_secrets::register(vw_secrets::SecretKind::VaultValue, secret);
    test_support::answering(&core.shared, move |_, _| {
        output_tests::mcp(json!({"status":"completed","action":"page.read","data":{
        "url":"https://example.com/","coverage":{"complete":false},"structure":[{"lists":[{"items":[secret]}]}]}}))
    });
    let mut request = json!({"sessionId":"t","action":"page.read","args":{},"saveTo":"csv","extract":{"kind":"list"}});
    let first = gateway::call(&core.shared, "s", "chrome_execute", request.clone())
        .await
        .unwrap();
    let first = &first["vwMcpResult"]["structuredContent"];
    assert_eq!(first["untrustedPageContent"]["truncated"], true);
    for key in ["path", "collection"] {
        let bytes =
            std::fs::read(folder.join(first["savedToFile"][key].as_str().unwrap())).unwrap();
        assert!(vw_secrets::find(&bytes).is_none());
    }
    request["appendFrom"] = first["savedToFile"]["collection"].clone();
    request["extract"]["columns"] = json!([{"name":"Changed"}]);
    let failed = gateway::call(&core.shared, "s", "chrome_execute", request.clone())
        .await
        .unwrap();
    assert_eq!(
        failed["vwMcpResult"]["structuredContent"]["error"]["code"], "SAVE_FAILED",
        "{failed}"
    );
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 2);
    request["appendFrom"] = json!("../outside.json");
    let failed = gateway::call(&core.shared, "s", "chrome_execute", request)
        .await
        .unwrap();
    assert_eq!(
        failed["vwMcpResult"]["structuredContent"]["error"]["code"], "SAVE_FAILED",
        "{failed}"
    );
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 2);
    core.shutdown().await;
}

#[tokio::test]
async fn continuations_keep_the_first_data_row_and_refuse_symlink_collections() {
    let (core, folder) = read_output_tests::fixture("browser-data-continuation").await;
    test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            return output_tests::status(args);
        }
        output_tests::mcp(json!({"status":"completed","action":"page.read","data":{
            "coverage":{"complete":true},"structure":[{"tables":[{"rows":[["First data"]]}]}]}}))
    });
    let mut request = json!({"sessionId":"t","action":"page.read","args":{"cursor":"h-captured:rows"},"saveTo":"csv","extract":{"kind":"table"}});
    assert!(
        gateway::call(&core.shared, "s", "chrome_execute", request.clone())
            .await
            .unwrap_err()
            .contains("data row")
    );
    request["extract"]["header"] = json!(false);
    request["extract"]["columns"] = json!([{"name":"Item"}]);
    let saved = gateway::call(&core.shared, "s", "chrome_execute", request.clone())
        .await
        .unwrap();
    let saved = &saved["vwMcpResult"]["structuredContent"]["savedToFile"];
    let csv = std::fs::read_to_string(folder.join(saved["path"].as_str().unwrap())).unwrap();
    assert!(csv.contains("First data"));
    #[cfg(unix)]
    {
        let name = "browser-data-00000000000000000000000000000000.json";
        std::os::unix::fs::symlink(
            folder.join(saved["collection"].as_str().unwrap()),
            folder.join(name),
        )
        .unwrap();
        request["appendFrom"] = json!(name);
        let failed = gateway::call(&core.shared, "s", "chrome_execute", request)
            .await
            .unwrap();
        assert_eq!(
            failed["vwMcpResult"]["structuredContent"]["error"]["code"],
            "SAVE_FAILED"
        );
    }
    core.shutdown().await;
}
