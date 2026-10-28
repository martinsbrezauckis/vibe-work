//! Built-in browser discovery must offer only the implemented browser target.
//! Reads the shipped static MCP definitions; called by cargo test.
use serde_json::json;

#[test]
fn browser_targets_offer_only_builtin() {
    let mut output = Vec::new();
    vw_agent_tools::mcp::serve(
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n".as_slice(),
        &mut output,
        |_: &str, _: serde_json::Value| panic!("discovery needs no core"),
    )
    .unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let list = &reply["result"]["tools"];
    for name in ["browser_read", "browser_act", "browser_capture"] {
        let tool = list
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap();
        assert_eq!(
            tool["inputSchema"]["properties"]["target"]["enum"],
            json!(["builtin"]),
            "{name}"
        );
    }
}
