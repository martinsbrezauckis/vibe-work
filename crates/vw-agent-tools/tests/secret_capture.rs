//! Pins optional command capture discovery and its value-free input schema.
//! Uses the public tool list; run by Cargo with and without the optional vault feature.
#[test]
fn command_capture_is_advertised_only_when_enabled() {
    let list = vw_agent_tools::mcp::tool_list();
    let found = list
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "secret_capture_command");
    assert_eq!(
        found.is_some(),
        cfg!(feature = "agent-vault"),
        "command capture must be discoverable in the optional module"
    );
    if let Some(tool) = found {
        assert_eq!(
            tool["inputSchema"]["required"],
            serde_json::json!(["name", "program"])
        );
        assert!(tool["inputSchema"]["properties"]["value"].is_null());
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    }
}

#[cfg(feature = "agent-vault")]
#[test]
fn capture_parser_bounds_runtime_without_weakening_existing_secret_runs() {
    use serde_json::json;
    use vw_agent_tools::secretspec::{parse, parse_capture};
    let args = json!({"program":"sh","args":["create.sh"]});
    let capture = parse_capture(&args).unwrap();
    assert_eq!(capture.bindings, []);
    assert_eq!(capture.timeout_s, 20);
    assert!(parse(&args).is_err());
    for args in [
        json!(null),
        json!([]),
        json!({"program":"sh","cwd":false}),
        json!({"program":"sh","timeoutSeconds":31}),
        json!({"program":"sh","args":["-c","echo unsafe"]}),
    ] {
        assert!(parse_capture(&args).is_err());
    }
    let ordinary = parse(&json!({"program":"sh","args":["use.sh"],"purpose":"Use a named token","env":[{"name":"TOKEN","secret":"Named token"}]})).unwrap();
    assert_eq!(ordinary.timeout_s, 120);
    assert_eq!(ordinary.bindings.len(), 1);
}
