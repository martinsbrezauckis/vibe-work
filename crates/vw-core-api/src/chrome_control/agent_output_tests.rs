//! Structural/content boundary regression cases for connected-browser MCP answers.
//! Uses the shared adapter directly; cursor dispatch and frozen paging are covered by `vw_cca`.
use super::*;
pub(super) const ID: &str = "3f2a9c0e1b7d4a6f8e2c9b1a0d3e5f7a9b2c4d6e";
pub(super) const HIDDEN: &str = "[looks like a key: hidden]";

pub(super) fn answer(action: &str, data: &Value) -> Value {
    json!({"action":action,"operationId":ID,"receiptId":ID,"bindingId":ID,
        "sessionId":ID,"untrustedPageContent":{"data":data}})
}

#[test]
fn continuation_cursor_is_opaque_but_extra_strings_filter_on_every_stream() {
    let args = json!({"cursor":ID,"tabId":ID,"frameId":ID,"ref":ID,"readId":ID,
        "documentId":ID,"query":ID,"maxBytes":12000});
    for stream in [
        "refs",
        "partialMatches",
        "text",
        "details",
        "structure",
        "frames",
        "changes",
        "candidates",
        "helper",
        "future/stream~",
    ] {
        let mut value = answer(
            "page.read",
            &json!({"text":"API key", "continuation":{
            stream:{"action":"page.read", "args":args,"reason":ID}}}),
        );
        filter(&mut value, "chrome_execute").unwrap();
        let next = &value["untrustedPageContent"]["data"]["continuation"][stream];
        let mut expected = args.clone();
        for name in ["query", "tabId", "frameId", "ref", "readId", "documentId"] {
            expected[name] = json!(HIDDEN);
        }
        assert_eq!(next["args"], expected, "{stream}");
        assert_eq!(next["reason"], HIDDEN);
    }
}

#[test]
fn typed_ids_reference_arrays_and_change_rows_survive() {
    for (action, data, preserved, hidden) in [
        (
            "bookmarks.read",
            json!({"text":"API key", "id":ID,"ids":[ID],"parentId":ID,
            "children":[{"id":ID,"parentId":ID,"title":ID}],"items":[{"id":ID,"title":ID}]}),
            vec!["/id", "/parentId"],
            vec![
                "/ids/0",
                "/children/0/id",
                "/children/0/parentId",
                "/items/0/id",
                "/children/0/title",
                "/items/0/title",
            ],
        ),
        (
            "network.requests",
            json!({"text":"API key", "requests":[{"id":ID,"requestId":ID,"url":ID}]}),
            vec!["/requests/0/id", "/requests/0/requestId"],
            vec!["/requests/0/url"],
        ),
        (
            "page.read",
            json!({"text":"API key", "tab":{"id":ID,"title":ID},
            "details":[{"ref":ID,"field":"selectedOptions","encoding":"application/json","text":ID}],
            "changes":{"appeared":[{"ref":ID,"name":ID}],"changed":[{"ref":ID,"value":ID}],"disappeared":[ID]},
            "removedRefs":[ID], "mediaIds":[ID]}),
            vec![
                "/details/0/ref",
                "/changes/appeared/0/ref",
                "/changes/changed/0/ref",
                "/changes/disappeared/0",
                "/removedRefs/0",
            ],
            vec![
                "/tab/id",
                "/tab/title",
                "/details/0/text",
                "/changes/appeared/0/name",
                "/changes/changed/0/value",
                "/mediaIds/0",
            ],
        ),
    ] {
        let mut value = answer(action, &data);
        filter(&mut value, "chrome_execute").unwrap();
        for name in ["operationId", "sessionId"] {
            assert_eq!(value[name], ID, "{action}/{name}");
        }
        for name in ["receiptId", "bindingId"] {
            assert_eq!(value[name], HIDDEN, "{action}/{name}");
        }
        let data = &value["untrustedPageContent"]["data"];
        for path in preserved {
            assert_eq!(data.pointer(path).unwrap(), ID, "{action}{path}");
        }
        for path in hidden {
            assert_eq!(data.pointer(path).unwrap(), HIDDEN, "{action}{path}");
        }
    }
}

#[test]
fn saved_read_and_session_bindings_keep_direct_payload_ids() {
    for tool in ["chrome_execute", "chrome_session", "chrome_status"] {
        let mut value = json!({"operationId":ID,"action":"page.read",
            "untrustedPageContent":{"text":"API key", "readId":ID,"documentId":ID,
                "clientSessionId":ID,"bindingId":ID,"receiptId":ID,
                "continuation":{"frames":{"action":"page.read","args":{"cursor":ID}}}}});
        filter(&mut value, tool).unwrap();
        let data = &value["untrustedPageContent"];
        for name in ["readId", "documentId", "clientSessionId"] {
            assert_eq!(data[name], ID, "{tool}/{name}");
        }
        for name in ["bindingId", "receiptId"] {
            assert_eq!(data[name], HIDDEN, "{tool}/{name}");
        }
        assert_eq!(data["continuation"]["frames"]["args"]["cursor"], ID);
    }
}

#[test]
fn raw_and_webmcp_results_cannot_claim_binding_exemptions() {
    for action in ["raw.execute", "webmcp.call"] {
        let mut value = answer(
            action,
            &json!({"text":"API key", "id":ID,"ref":ID,"mediaId":ID,
            "page":{"readId":ID},"continuation":{"frames":{"args":{"cursor":ID}}}}),
        );
        filter(&mut value, "chrome_execute").unwrap();
        let data = &value["untrustedPageContent"]["data"];
        for path in [
            "/id",
            "/ref",
            "/mediaId",
            "/page/readId",
            "/continuation/frames/args/cursor",
        ] {
            assert_eq!(data.pointer(path).unwrap(), HIDDEN, "{action}{path}");
        }
    }
}

#[test]
fn mcp_resource_addresses_stay_usable_but_resource_text_is_filtered() {
    // MAIN-NOTE: URI provenance is unproven, so the ordinary control uses a short opaque ID.
    let uri = "fixture://asset-7";
    let mut value = json!({"vwMcpResult":{"structuredContent":answer("page.read", &json!({"text":"API key"})),
        "content":[{"type":"resource","resource":{"uri":uri,"mimeType":"application/json","text":ID}},
            {"type":"resource_link","uri":uri,"mimeType":"application/octet-stream","name":ID,"description":ID}]}});
    filter(&mut value, "chrome_execute").unwrap();
    let content = value["vwMcpResult"]["content"].as_array().unwrap();
    let resource = content
        .iter()
        .find(|row| row["type"] == "resource")
        .unwrap();
    assert_eq!(resource["resource"]["uri"], uri);
    assert_eq!(resource["resource"]["text"], HIDDEN);
    let link = content
        .iter()
        .find(|row| row["type"] == "resource_link")
        .unwrap();
    assert_eq!(link["uri"], uri);
    assert_eq!(link["mimeType"], "application/octet-stream");
    assert_eq!(link["name"], HIDDEN);
    assert_eq!(link["description"], HIDDEN);
}

#[test]
fn generated_action_binding_arguments_remain_addressable_in_answers() {
    let catalog: Value = serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap();
    let mut checked = std::collections::BTreeSet::new();
    for action in catalog["actions"].as_array().unwrap() {
        let id = action["id"].as_str().unwrap();
        if id == "raw.execute" || id.starts_with("webmcp.") {
            continue;
        }
        for (field, schema) in action["inputSchema"]["properties"].as_object().unwrap() {
            if !(field.ends_with("Id")
                || field.ends_with("Ids")
                || ["id", "ids", "ref", "toRef", "cursor"].contains(&field.as_str()))
            {
                continue;
            }
            let binding = match schema["type"].as_str() {
                Some("integer") => json!(2_147_483_647),
                Some("array") if schema["items"]["type"] == "integer" => json!([2_147_483_647]),
                Some("array") => json!([ID]),
                _ => json!(ID),
            };
            let mut value = answer(id, &json!({"text":"API key",field:binding}));
            filter(&mut value, "chrome_execute").unwrap();
            let expected = match field.as_str() {
                "ids" => json!([HIDDEN]),
                "toRef" | "dialogId" | "downloadId" if binding.is_string() => json!(HIDDEN),
                _ => binding,
            };
            assert_eq!(
                value["untrustedPageContent"]["data"][field], expected,
                "{id}/{field}"
            );
            checked.insert(field.clone());
        }
    }
    assert_eq!(
        checked.len(),
        17,
        "review new generated binding argument types: {checked:?}"
    );
}

#[test]
fn nested_receipts_and_protocol_diagnostics_keep_their_bindings() {
    let mut step = answer(
        "page.read",
        &json!({"text":"API key", "error":{"code":ID,"message":ID}}),
    );
    step["error"] = json!({"code":ID,"message":ID});
    step["selectedProfile"] = json!({"profileId":ID,"name":ID});
    step["promptGuard"] = json!({"flagged":true,"signals":[{"channel":"Browser page", "signal":"instructionOverride","confidence":"high"}]});
    let mut value = json!({"steps":[{"step":1,"result":step}]});
    filter(&mut value, "chrome_execute").unwrap();
    let step = &value["steps"][0]["result"];
    assert_eq!(step["error"]["code"], ID);
    assert_eq!(step["error"]["message"], HIDDEN);
    assert_eq!(step["selectedProfile"]["profileId"], ID);
    assert_eq!(step["selectedProfile"]["name"], HIDDEN);
    assert_eq!(
        step["promptGuard"]["signals"][0]["signal"],
        "instructionOverride"
    );
    let error = &step["untrustedPageContent"]["data"]["error"];
    assert_eq!(error["code"], ID);
    assert_eq!(error["message"], HIDDEN);
}

async fn safe_page_core(profile: &str) -> crate::core::Core {
    let core =
        super::super::output_tests::core(&format!("structural-safe-{}", profile.replace(':', "-")))
            .await;
    *core
        .shared
        .chrome_control
        .owner
        .edge_pairing
        .lock()
        .unwrap() = (super::super::PairingState::Paired, None);
    core.shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap()
        .insert("s".into(), profile.into());
    core.shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("t", "s", "active", profile, None);
    core
}

#[tokio::test]
async fn chrome_and_edge_large_safe_answers_page_through_both_cursor_layers() {
    use super::super::{agent_browser, gateway, output_tests, test_support};
    const READ: &str = "Read0123456789AbCdEfGhIjKlMnOpQrStUvWxYz";
    for profile in ["chrome:profile-0", "edge:profile-0"] {
        if profile.starts_with("edge:") && !cfg!(feature = "your-edge") {
            continue;
        }
        let core = safe_page_core(profile).await;
        let data = json!({"readId":READ,"documentId":ID,"text":"Ordinary visible text. ".repeat(2000),
            "continuation":{"frames":{"action":"page.read","args":{"cursor":format!("{READ}:frames:0:0"),"tabId":7}}}});
        let captured = data.clone();
        let seen = test_support::answering(&core.shared, move |tool, args| {
            if tool == "chrome_status" {
                let mut result = output_tests::status(args);
                let status = &mut result["structuredContent"];
                status["profiles"][0]["profileId"] = json!(profile);
                status["profiles"][0]["extensionCapabilities"] = json!(["page.read.cursor"]);
                status["sessions"][0]["profileId"] = json!(profile);
                result
            } else {
                let data = if args["args"].get("cursor").is_some() {
                    assert_eq!(args["args"], captured["continuation"]["frames"]["args"]);
                    json!({"readId":READ,"text":"Remaining page"})
                } else {
                    captured.clone()
                };
                output_tests::mcp(json!({"profileId":profile,"sessionId":"t","operationId":ID,
                    "action":"page.read","status":"completed","data":data}))
            }
        });
        let mut request = json!({"sessionId":"t","action":"page.read","args":{}});
        assert_eq!(
            agent_browser::selected(&core.shared, "s", "chrome_execute", &request).id(),
            profile.split_once(':').unwrap().0
        );
        let mut encoded = String::new();
        let mut pages = 0;
        loop {
            pages += 1;
            assert!(pages < 64);
            let mut answer = gateway::call(&core.shared, "s", "chrome_execute", request.clone())
                .await
                .unwrap();
            filter(&mut answer, "chrome_execute").unwrap();
            let body = &answer["vwMcpResult"]["structuredContent"];
            assert_eq!(body["untrustedPageContent"]["frozen"], true);
            encoded.push_str(
                body["untrustedPageContent"]["details"]["text"]
                    .as_str()
                    .unwrap(),
            );
            if body["continuation"].is_null() {
                break;
            }
            request["args"] = body["continuation"]["args"].clone();
        }
        assert!(pages > 1);
        let original: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(original["untrustedPageContent"]["data"], data);
        request["args"] =
            original["untrustedPageContent"]["data"]["continuation"]["frames"]["args"].clone();
        let mut next = gateway::call(&core.shared, "s", "chrome_execute", request)
            .await
            .unwrap();
        filter(&mut next, "chrome_execute").unwrap();
        assert_eq!(
            next["vwMcpResult"]["structuredContent"]["status"],
            "completed"
        );
        assert_eq!(
            next["vwMcpResult"]["structuredContent"]["untrustedPageContent"]["data"]["readId"],
            READ
        );
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
}

#[test]
fn unproduced_notification_and_extension_addresses_remain_content() {
    for (action, data, paths) in [
        (
            "notifications.create",
            json!({"id":ID,"title":"API key","message":ID}),
            vec!["/id"],
        ),
        (
            "notifications.list",
            json!({"ids":[ID],"text":"API key"}),
            vec!["/ids/0"],
        ),
        (
            "extensions.list",
            json!([{"id":ID,"name":"API key","description":ID}]),
            vec!["/0/id"],
        ),
    ] {
        let mut value = answer(action, &data);
        filter(&mut value, "chrome_execute").unwrap();
        let data = &value["untrustedPageContent"]["data"];
        for path in paths {
            assert_eq!(data.pointer(path).unwrap(), HIDDEN, "{action}{path}");
        }
        if action == "notifications.create" {
            assert_eq!(data["message"], HIDDEN);
        }
        if action == "extensions.list" {
            assert_eq!(data[0]["description"], HIDDEN);
        }
    }
}

#[test]
fn connection_identity_remains_protocol_metadata() {
    let mut value = json!({"untrustedPageContent":{"text":"API key", "ownerPublicKey":ID,
        "ownerFingerprint":ID,"owner":{"key":ID,"fingerprint":ID}}});
    filter(&mut value, "chrome_status").unwrap();
    let data = &value["untrustedPageContent"];
    for path in [
        "/ownerPublicKey",
        "/ownerFingerprint",
        "/owner/key",
        "/owner/fingerprint",
    ] {
        assert_eq!(data.pointer(path).unwrap(), ID, "{path}");
    }
}

#[test]
fn fixed_action_names_remain_usable() {
    let own = super::super::output_status::unknown_action("missing.action").unwrap();
    let mut value = json!({"vwMcpResult":own});
    filter(&mut value, "chrome_status").unwrap();
    assert_eq!(
        value["vwMcpResult"]["structuredContent"],
        own["structuredContent"]
    );
}

#[test]
fn observation_protocol_codes_do_not_count_as_page_content() {
    let mut value = answer(
        "page.read",
        &json!({"text":"API key", "mimeType":"application/octet-stream",
        "coverage":{"complete":false,"reason":"captured_content_remaining","sourceLimits":["captured_frames_incomplete"]},
        "verification":"file_list_read_back","resetReason":"document_changed","matcher":"deterministic"}),
    );
    let mut expected = value.clone();
    for name in ["receiptId", "bindingId"] {
        expected[name] = json!(HIDDEN);
    }
    filter(&mut value, "chrome_execute").unwrap();
    assert_eq!(value, expected);
}
