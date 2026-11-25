//! Server/page-origin regressions for the connected-browser answer filter.
//! Calls the production adapter; structural paging and access modes also run in the whole `vw_cca` target.
use super::tests::{HIDDEN, ID, answer};
use super::*;

#[test]
fn server_mime_is_content_in_network_rows_and_bodies() {
    for action in [
        "network.list",
        "network.body",
        "downloads.list",
        "media.read",
    ] {
        for mime in [
            ID.to_owned(),
            format!("application/{ID}"),
            format!("{ID}/plain"),
        ] {
            for data in [
                json!({"mimeType":mime}),
                json!({"items":[{"mimeType":mime}]}),
                json!({"requests":[{"mimeType":mime}]}),
            ] {
                let mut value = answer(action, &data);
                value["untrustedPageContent"]["text"] = json!("API key");
                filter(&mut value, "chrome_execute").unwrap();
                let data = &value["untrustedPageContent"]["data"];
                let mime = data
                    .get("mimeType")
                    .or_else(|| data.pointer("/items/0/mimeType"))
                    .or_else(|| data.pointer("/requests/0/mimeType"))
                    .unwrap();
                assert_eq!(mime, HIDDEN, "{action}: {data}");
            }
        }
    }
}

#[test]
fn mime_metadata_requires_a_parameter_free_token_pair() {
    for mime in [
        "",
        "text",
        "/plain",
        "text/",
        "text/plain/extra",
        "text/plain; charset=utf-8",
        "text/ plain",
        "text/π",
        "text/plain\r\nX: y",
    ] {
        let mut value = answer("network.body", &json!({"text":"API key","mimeType":mime}));
        filter(&mut value, "chrome_execute").unwrap();
        assert_eq!(
            value["untrustedPageContent"]["data"]["mimeType"], HIDDEN,
            "{mime:?}"
        );
    }
    for data in [json!({}), json!({"mimeType":null}), json!({"mimeType":7})] {
        let mut value = answer("network.body", &data);
        filter(&mut value, "chrome_execute").unwrap();
        assert_eq!(value["untrustedPageContent"]["data"], data);
    }
}

#[test]
fn ordinary_mime_values_remain_exact() {
    for mime in [
        "text/html",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "image/svg+xml",
        "application/octet-stream",
        "text/x!#$%&'*+-.^_`|~",
    ] {
        let mut value = answer(
            "network.list",
            &json!({"text":"API key", "items":[{"mimeType":mime}]}),
        );
        filter(&mut value, "chrome_execute").unwrap();
        assert_eq!(
            value["untrustedPageContent"]["data"]["items"][0]["mimeType"],
            mime
        );
    }
}

#[test]
fn mcp_block_mime_cannot_bypass_the_content_filter() {
    for kind in ["image", "audio", "resource", "resource_link"] {
        let mut block =
            json!({"type":kind,"mimeType":ID,"data":"YWJj","uri":"fixture://asset","mediaId":ID});
        if kind == "resource" {
            block["resource"] =
                json!({"uri":"fixture://asset","mediaId":ID,"mimeType":ID,"text":ID});
        }
        let mut value = json!({"vwMcpResult":{"structuredContent":answer("network.body", &json!({"text":"API key"})),"content":[block]}});
        filter(&mut value, "chrome_execute").unwrap();
        let block = &value["vwMcpResult"]["content"][1];
        assert_eq!(block["mimeType"], HIDDEN, "{kind}");
        assert_eq!(block["mediaId"], HIDDEN, "{kind}");
        if kind == "resource" {
            assert_eq!(block["resource"]["mimeType"], HIDDEN);
        }
        if kind == "image" || kind == "audio" {
            assert_eq!(block["data"], "YWJj");
        }
    }
}

#[test]
fn page_strings_in_every_returned_content_field_are_hidden() {
    for field in [
        "url",
        "title",
        "name",
        "selector",
        "filename",
        "suggestedFilename",
        "mime",
        "method",
        "type",
        "text",
        "value",
        "description",
        "label",
        "message",
        "error",
        "origin",
        "src",
        "href",
        "alt",
        "placeholder",
        "downloadName",
        "role",
        "tag",
        "inputType",
        "context",
        "frameOrigin",
        "matchedText",
        "textPreview",
        "html",
        "cookie",
        "header",
        "body",
    ] {
        for container in [
            "items",
            "refs",
            "frames",
            "elements",
            "downloads",
            "requests",
            "files",
            "media",
            "tabs",
            "bookmarks",
        ] {
            let mut value = answer("page.read", &json!({"keyLike":true,container:[{field:ID}]}));
            filter(&mut value, "chrome_execute").unwrap();
            assert_eq!(
                value["untrustedPageContent"]["data"][container][0][field], HIDDEN,
                "{container}.{field}"
            );
        }
    }
    let mut value = answer(
        "network.headers",
        &json!({"text":"API key", "request":{"Authorization":ID},"response":{"X-Trace":ID}}),
    );
    filter(&mut value, "chrome_execute").unwrap();
    assert_eq!(
        value["untrustedPageContent"]["data"]["request"]["Authorization"],
        HIDDEN
    );
    assert_eq!(
        value["untrustedPageContent"]["data"]["response"]["X-Trace"],
        HIDDEN
    );
}

#[test]
fn mime_filter_keeps_key_context_for_sibling_page_values() {
    let mut value = answer(
        "network.list",
        &json!({"items":[{"mimeType":"text/secret; invalid", "url":ID}]}),
    );
    filter(&mut value, "chrome_execute").unwrap();
    let row = &value["untrustedPageContent"]["data"]["items"][0];
    assert_eq!(row["mimeType"], HIDDEN);
    assert_eq!(row["url"], HIDDEN);
}

#[test]
fn mime_grammar_never_blesses_a_long_token_or_known_prefix() {
    for token in [
        "aaaaaaaaaaaaaaaa",
        "1234567890123456",
        "0123456789abcdef",
        "sk-12345678901234",
        "eyJabcde.eyJabcde.abcdefgh",
    ] {
        let mut value = answer(
            "network.body",
            &json!({"text":"API key","mimeType":format!("application/{token}")}),
        );
        filter(&mut value, "chrome_execute").unwrap();
        assert_eq!(
            value["untrustedPageContent"]["data"]["mimeType"], HIDDEN,
            "{token}"
        );
    }
}

#[test]
fn resource_uris_are_content_until_an_app_producer_is_proven() {
    let uri = format!("https://example.test/{ID}");
    let mut value = json!({"vwMcpResult":{"structuredContent":answer("page.read", &json!({"text":"API key"})),
        "content":[{"type":"resource","resource":{"uri":uri,"text":"ordinary"}},
            {"type":"resource_link","uri":uri,"name":"ordinary"}]}});
    filter(&mut value, "chrome_execute").unwrap();
    let blocks = &value["vwMcpResult"]["content"];
    assert_eq!(
        blocks[1]["resource"]["uri"],
        format!("https://example.test/{HIDDEN}")
    );
    assert_eq!(blocks[2]["uri"], format!("https://example.test/{HIDDEN}"));
}

#[test]
fn unproduced_aliases_do_not_gain_protocol_exemptions() {
    for field in [
        "bindingId",
        "receiptId",
        "captureId",
        "elementId",
        "nextCursor",
        "targetId",
        "toRef",
        "downloadId",
        "dialogId",
    ] {
        let mut value = answer("page.read", &json!({"text":"API key", field:ID}));
        filter(&mut value, "chrome_execute").unwrap();
        assert_eq!(
            value["untrustedPageContent"]["data"][field], HIDDEN,
            "{field}"
        );
    }
}

#[test]
fn fixed_mime_literals_do_not_allow_suffixes_or_parameters() {
    for mime in [
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ] {
        for suffix in [";charset=x".to_owned(), format!(".{ID}")] {
            let mut value = answer(
                "network.body",
                &json!({"text":"API key","mimeType":format!("{mime}{suffix}")}),
            );
            filter(&mut value, "chrome_execute").unwrap();
            assert_eq!(value["untrustedPageContent"]["data"]["mimeType"], HIDDEN);
        }
    }
}

#[test]
fn continuation_page_arguments_and_unproduced_lists_are_content() {
    let mut value = answer(
        "page.read",
        &json!({"text":"API key","mediaIds":[ID],"tabIds":[ID],"refs":[ID],
        "continuation":{"frames":{"action":"page.read","args":{"cursor":ID,"tabId":7,"query":ID,"status":ID,"action":ID}}}}),
    );
    filter(&mut value, "chrome_execute").unwrap();
    let data = &value["untrustedPageContent"]["data"];
    assert_eq!(data["continuation"]["frames"]["args"]["query"], HIDDEN);
    assert_eq!(data["continuation"]["frames"]["args"]["status"], HIDDEN);
    assert_eq!(data["continuation"]["frames"]["args"]["action"], HIDDEN);
    assert_eq!(data["continuation"]["frames"]["args"]["cursor"], ID);
    assert_eq!(data["mediaIds"][0], HIDDEN);
    assert_eq!(data["tabIds"][0], HIDDEN);
    assert_eq!(data["refs"][0], HIDDEN);
}

#[test]
fn firefox_shaped_network_rows_and_body_filter_server_mime() {
    for (action, data) in [
        (
            "network.list",
            json!({"items":[{"id":"7","method":"GET","url":"https://example.test/keys","type":"document","status":200,"mimeType":ID,"finished":true}],"total":1,"nextOffset":null}),
        ),
        (
            "network.body",
            json!({"request":"bidi-request-7","mimeType":format!("application/{ID}")}),
        ),
    ] {
        let mut value = answer(action, &data);
        value["profileId"] = json!("firefox:agent");
        value["untrustedPageContent"]["text"] = json!("API key");
        filter(&mut value, "chrome_execute").unwrap();
        let data = &value["untrustedPageContent"]["data"];
        let mime = data
            .get("mimeType")
            .or_else(|| data.pointer("/items/0/mimeType"))
            .unwrap();
        assert_eq!(mime, HIDDEN, "{action}: {data}");
        assert_eq!(value["profileId"], "firefox:agent");
    }
}

#[test]
fn unproduced_bare_addresses_and_inherited_ids_are_content() {
    for container in [
        "refs",
        "frames",
        "tabs",
        "elements",
        "target",
        "profiles",
        "sessions",
        "media",
        "files",
        "currentFiles",
        "nodes",
        "binding",
        "bindings",
        "receipt",
        "receipts",
        "tab",
        "frame",
        "element",
        "browserTab",
        "windows",
        "window",
        "groups",
        "group",
        "points",
        "point",
        "selectedProfile",
        "profileChoices",
        "dialogs",
    ] {
        let mut value = answer(
            "page.read",
            &json!({"text":"API key",container:[{"id":ID,"parentId":ID,"ids":[ID]}]}),
        );
        filter(&mut value, "chrome_execute").unwrap();
        let row = &value["untrustedPageContent"]["data"][container][0];
        assert_eq!(row["id"], HIDDEN, "{container}");
        assert_eq!(row["parentId"], HIDDEN, "{container}");
        assert_eq!(row["ids"][0], HIDDEN, "{container}");
    }
    for action in [
        "notifications.create",
        "notifications.list",
        "extensions.list",
        "media.read",
        "tabs.list",
        "window.resize",
        "windows.list",
    ] {
        let mut value = answer(action, &json!({"text":"API key","id":ID,"ids":[ID]}));
        filter(&mut value, "chrome_execute").unwrap();
        assert_eq!(
            value["untrustedPageContent"]["data"]["id"], HIDDEN,
            "{action}"
        );
        assert_eq!(
            value["untrustedPageContent"]["data"]["ids"][0], HIDDEN,
            "{action}"
        );
    }
    let mut value = answer(
        "network.list",
        &json!({"text":"API key","items":[{"id":ID,"children":[{"id":ID}]}]}),
    );
    filter(&mut value, "chrome_execute").unwrap();
    let row = &value["untrustedPageContent"]["data"]["items"][0];
    assert_eq!(row["id"], ID);
    assert_eq!(row["children"][0]["id"], HIDDEN);
}

#[test]
fn proven_bare_protocol_addresses_stay_exact() {
    for (container, row) in [
        ("requests", json!({"id":ID,"url":ID})),
        ("downloads", json!({"id":ID,"name":ID})),
        ("bookmarks", json!({"id":ID,"parentId":ID,"title":ID})),
        ("dialog", json!({"id":ID,"message":ID})),
    ] {
        let mut value = answer("page.read", &json!({"text":"API key",container:row}));
        filter(&mut value, "chrome_execute").unwrap();
        let row = &value["untrustedPageContent"]["data"][container];
        assert_eq!(row["id"], ID, "{container}");
        if container == "bookmarks" {
            assert_eq!(row["parentId"], ID);
        }
        for name in ["url", "name", "title", "message"] {
            if row.get(name).is_some() {
                assert_eq!(row[name], HIDDEN, "{container}/{name}");
            }
        }
    }
}
