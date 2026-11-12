//! Another same-user program (another chat's Full-access agent) cannot read a chat's tool credential
//! from the running MCP helper's environment on Linux.
#![cfg(target_os = "linux")]
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn mcp_helper_environment_is_unreadable_to_same_user_programs() {
    let marker = "vwtestcredmcpdumpmarker";
    let mut child = Command::new(env!("CARGO_BIN_EXE_vw-agent-tools"))
        .arg("mcp")
        .env("VW_AGENT_ENDPOINT", "127.0.0.1:9")
        .env("VW_AGENT_CREDENTIAL", marker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the MCP helper");
    // One answered request proves the helper is past its startup (and so past the hardening).
    let mut stdin = child.stdin.take().expect("stdin");
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}"#
    )
    .expect("write initialize");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut line)
        .expect("read initialize answer");
    assert!(line.contains("\"id\":1"), "no initialize answer: {line}");
    let environ = std::fs::read(format!("/proc/{}/environ", child.id()));
    drop(stdin);
    let _ = child.wait();
    match environ {
        Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied),
        Ok(bytes) => panic!(
            "environment readable ({} bytes, marker present: {})",
            bytes.len(),
            String::from_utf8_lossy(&bytes).contains(marker)
        ),
    }
}
