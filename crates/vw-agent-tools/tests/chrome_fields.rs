//! Browser session and execute calls carry only the fields their published schemas list. An unlisted field (such as
//! a Stop option that would close tabs) is refused before anything reaches the core; listed fields pass unchanged.
//! PREDICT (before the check): `chrome_session` stop with `closeOwnedTabs` and `chrome_execute` with an extra top-level
//! field were forwarded to the core as sent.
use serde_json::{Value, json};

#[test]
fn unlisted_browser_session_and_execute_fields_never_reach_the_core() {
    for (tool, arguments) in [
        (
            "chrome_session",
            json!({"action":"stop","sessionId":"s1","closeOwnedTabs":true}),
        ),
        (
            "chrome_session",
            json!({"action":"open","profileId":"chrome","closeOwnedTabs":true}),
        ),
        (
            "chrome_execute",
            json!({"sessionId":"s1","action":"page.read","args":{},"closeOwnedTabs":true}),
        ),
        ("chrome_status", json!({"extra":1})),
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
        assert!(calls.is_empty(), "unlisted field forwarded: {calls:?}");
        assert_eq!(reply["error"]["code"], -32602, "{reply}");
    }
}

#[test]
fn listed_browser_session_and_execute_fields_are_forwarded_unchanged() {
    for (tool, arguments) in [
        ("chrome_session", json!({"action":"stop","sessionId":"s1"})),
        (
            "chrome_session",
            json!({"action":"open","profileId":"chrome","purpose":"Trip","route":"extension","tabId":3}),
        ),
        (
            "chrome_execute",
            json!({"sessionId":"s1","action":"page.read","args":{},"saveTo":"inline"}),
        ),
        ("chrome_status", json!({"action":"page.read"})),
    ] {
        let input = format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":arguments.clone()}})
        );
        let mut output = Vec::new();
        let mut calls = Vec::new();
        vw_agent_tools::mcp::serve(input.as_bytes(), &mut output, |name: &str, args| {
            calls.push((name.to_owned(), args));
            Ok(json!({}))
        })
        .unwrap();
        assert!(output.starts_with(b"{"));
        assert_eq!(calls, vec![(tool.to_owned(), arguments)]);
    }
}
