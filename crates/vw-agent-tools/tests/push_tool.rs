//! Push discovery contract, called by Cargo; depends on the compiled MCP catalog.
//! PREDICT: push exposes branch, remote and result inputs without force or credential inputs.

#[test]
fn push_is_discoverable_without_force_or_credential_inputs() {
    let list = vw_agent_tools::mcp::tool_list();
    let push = list
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "push")
        .expect("app-owned push tool must be discoverable");
    let schema = &push["inputSchema"];
    assert_eq!(schema["additionalProperties"], false);
    let properties = schema["properties"].as_object().unwrap();
    assert!(properties.contains_key("branch"));
    assert!(properties.contains_key("remote"));
    assert!(!properties.contains_key("force"));
    assert!(!properties.contains_key("token"));
}

// PREDICT: a missing push reply preserves uncertainty and supplies a read-only recovery call.
#[test]
fn push_lost_reply_never_claims_nothing_ran_and_names_result_collection() {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&mut stream).read_line(&mut line).unwrap();
        stream.write_all(b"unreadable reply\n").unwrap();
    });
    let reason = vw_agent_tools::wire::call(
        &address.to_string(),
        "fixture-only",
        "push",
        serde_json::json!({}),
        std::time::Duration::from_secs(2),
    )
    .unwrap_err();
    server.join().unwrap();
    assert!(
        reason.starts_with(vw_agent_tools::wire::UNKNOWN_OUTCOME),
        "{reason}"
    );
    assert!(
        reason.contains("pushId") && reason.contains("latest"),
        "{reason}"
    );
}
