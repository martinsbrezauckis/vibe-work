//! App-owned structured export schema shared by built-in reads and connected-browser discovery.
//! The core consumes export fields before sending the existing page.read arguments to a helper.
use serde_json::{Map, Value, json};
pub(crate) const DESCRIPTION: &str = " Structured extraction uses extract:{kind:table|list|links} with saveTo:csv|xlsx. It writes a verified file and collection in the chat folder. appendFrom takes a previous collection filename to add pages; deduplicate:true removes identical rows. It does not advance pages.";

pub(crate) fn properties(connected: bool) -> Map<String, Value> {
    let column = json!({"type":"object","properties":{
        "name":{"type":"string","maxLength":200},
        "selector":{"type":"string","maxLength":2000},
        "attribute":{"enum":["text","href"]}
    },"required":["name"],"additionalProperties":false});
    let mut properties=json!({
        "saveTo":{"enum":["csv","xlsx"],"description":"Save rendered rows in a new CSV or spreadsheet in this chat's folder."},
        "extract":{"type":"object","properties":{
            "kind":{"enum":["table","list","links"]},
            "selector":{"type":"string","maxLength":2000,"description":"Built-in only: CSS for table roots or repeated items. Defaults to table, li/[role=listitem], or a[href]."},
            "index":{"type":"integer","minimum":0,"maximum":63,"description":"Built-in only: table index in main-document then frame order."},
            "header":{"type":"boolean","description":"Connected tables: first row is a header (default true); false for a continuation without headers."},
            "columns":{"type":"array","maxItems":32,"items":column,"description":"Named columns. Tables rename by position. Built-in lists/links support relative CSS and text/href values; connected reads support names only."}
        },"required":["kind"],"additionalProperties":false},
        "appendFrom":{"type":"string","description":"Previous export's collection filename, in this chat folder. Creates a new collection and file."},
        "deduplicate":{"type":"boolean","default":false,"description":"Remove identical rows when accumulating repeated or overlapping pages."}
    }).as_object().cloned().unwrap_or_default();
    if connected {
        properties["saveTo"]["enum"] = json!(["inline", "file", "csv", "xlsx"]);
        properties["saveTo"]["description"] = json!(
            "Single page.read: inline (default), file (raw JSON), csv or xlsx (requires extract). Table extraction requires args.ref for one table subtree; cursor continues its captured rows. A read saves only its captured coverage."
        );
    }
    properties
}

pub(crate) fn extend(tools: &mut [Value]) {
    for tool in tools {
        if tool["name"] == "browser_read" {
            if let Some(properties) = tool["inputSchema"]["properties"].as_object_mut() {
                properties.insert("ref".into(), json!({"type":"string","description":"Read only this observed DOM subtree in its document, including open shadow roots. Nested frame documents have their own refs. Omit find, selector, extract and continuation cursors; textPage pages subtree text. A replaced root refuses."}));
                properties.extend(self::properties(false));
            }
            tool["description"] = json!(format!(
                "{}{DESCRIPTION}",
                tool["description"].as_str().unwrap_or_default()
            ));
        }
    }
}
