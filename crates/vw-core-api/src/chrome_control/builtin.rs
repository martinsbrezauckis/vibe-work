//! Built-in browser action aliases for Chrome's permission table.
//! The tool gate uses these only for Accept edits; vault and tab-close consent stay in their own handlers.
use serde_json::{Value, json};
use vw_ask::chrome::Tier;

pub(crate) fn accepts_edits(args: &Value) -> bool {
    let action = match args["action"].as_str().unwrap_or_default() {
        "navigate" => "page.navigate",
        "back" => "page.back",
        "forward" => "page.forward",
        "reload" => "page.reload",
        "click" => "page.click",
        "hover" => "page.hover",
        "type" => "page.type",
        "select" => "page.select",
        "check" => "page.check",
        "press" => "page.key",
        "scroll" => "page.scroll",
        "wait" => "page.wait",
        "list_tabs" => "tabs.list",
        "drag" => "page.drag",
        "upload" => "files.upload",
        _ => return false,
    };
    super::mapping::required("chrome_execute", &json!({"action":action,"args":args}))
        .is_some_and(|tier| tier <= Tier::AcceptEdits)
}
