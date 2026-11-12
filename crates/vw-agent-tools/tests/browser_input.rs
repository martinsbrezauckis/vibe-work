//! Built-in browser inputs reject unadvertised fields before any call reaches the executor.
//! Exercises the in-memory MCP server with a recording forwarder; no browser is opened.
use serde_json::{Value, json};

#[test]
fn unknown_browser_fields_never_reach_the_executor() {
    for (tool, arguments) in [
        (
            "browser_act",
            json!({"action":"navigate","url":"https://executed.example","args":{"url":"https://claimed.example"}}),
        ),
        ("browser_act", json!({"action":"list_tabs","extra":true})),
        (
            "browser_read",
            json!({"screenshot":true,"args":{"screenshot":false}}),
        ),
        ("browser_read", json!({"extra":null})),
    ] {
        let input = format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":arguments}})
        );
        let mut output = Vec::new();
        let mut calls = Vec::new();
        vw_agent_tools::mcp::serve(input.as_bytes(), &mut output, |name: &str, args| {
            calls.push((name.to_owned(), args));
            Ok(json!({}))
        })
        .unwrap();
        let reply: Value = serde_json::from_slice(&output).unwrap();
        assert!(
            calls.is_empty(),
            "unvalidated {tool} was forwarded: {calls:?}"
        );
        assert_eq!(reply["error"]["code"], -32602, "{reply}");
    }
}

#[test]
fn advertised_browser_fields_are_forwarded_unchanged() {
    for (tool, mut arguments) in [
        (
            "browser_act",
            json!({"action":"navigate","url":"https://executed.example","target":"builtin","tab":"new"}),
        ),
        (
            "browser_act",
            json!({"action":"click","x":10,"y":20,"ref":"e00000001"}),
        ),
        (
            "browser_act",
            json!({"action":"fill","login":"saved","ref":"e00000001"}),
        ),
        (
            "browser_read",
            json!({"tab":"current","screenshot":true,"find":"text","textPage":2,"maxRefs":10,"maxBytes":4000}),
        ),
    ] {
        arguments[vw_agent_tools::wire::tools::RECEIPT_FIELD] = json!("receipt");
        let input = format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":arguments}})
        );
        let mut output = Vec::new();
        let mut calls = Vec::new();
        vw_agent_tools::mcp::serve(input.as_bytes(), &mut output, |name: &str, args| {
            calls.push((name.to_owned(), args));
            Ok(json!({}))
        })
        .unwrap();
        assert_eq!(calls, [(tool.to_owned(), arguments)]);
    }
}

#[test]
fn ordered_steps_validate_every_child_before_forwarding() {
    use vw_agent_tools::browser_input::validate;
    assert!(
        validate(
            "browser_act",
            &json!({"steps":[{"action":"list_tabs"},{"action":"window.list"}]})
        )
        .is_ok()
    );
    for args in [
        json!({"steps":[]}),
        json!({"steps":vec![json!({"action":"list_tabs"});17]}),
        json!({"action":"list_tabs","steps":[{"action":"list_tabs"}]}),
        json!({"steps":[{"steps":[{"action":"list_tabs"}]}]}),
        json!({"steps":[{"action":"list_tabs","extra":true}]}),
        json!({"steps":[{"action":"not_an_action"}]}),
    ] {
        assert!(validate("browser_act", &args).is_err(), "{args}");
    }
}
