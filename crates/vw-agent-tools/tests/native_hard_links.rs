//! The shipped folder-hook binary sends native grep directory targets for the core's identity check.
//! Uses only disposable files and a synthetic loopback core reply.

#![cfg(any(unix, windows))]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

#[test]
fn grep_directory_asks_the_core_about_linked_files() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("app-data/vibe-home");
    let project = root.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let protected = root.path().join("app-data/synthetic.txt");
    std::fs::write(&protected, "synthetic").unwrap();
    std::fs::hard_link(&protected, project.join("alias.txt")).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = root.path().join("hook-endpoint.json");
    std::fs::write(
        &endpoint,
        json!({
            "endpoint":listener.local_addr().unwrap().to_string(), "credential":"synthetic-hook"
        })
        .to_string(),
    )
    .unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut line = String::new();
                    BufReader::new(&stream).read_line(&mut line).unwrap();
                    let request: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(request["tool"], "native_file_identity");
                    assert_eq!(request["args"]["search"], true);
                    stream.write_all(b"{\"ok\":{\"allowed\":false}}\n").unwrap();
                    return true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        }
        false
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_vw-agent-tools"))
        .args(["guard", "folder", "--endpoint-file"])
        .arg(&endpoint)
        .env("VIBE_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let payload = json!({"session_id":"test","cwd":project,"tool_name":"file_system.grep",
        "tool_call_id":"grep-dir","tool_input":{"path":".","pattern":"synthetic"}});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    assert_eq!(answer["decision"], "deny", "{answer}");
    assert!(
        answer["reason"]
            .as_str()
            .unwrap()
            .contains("vault or app state")
    );
    assert!(server.join().unwrap(), "the core was not consulted");
}

#[test]
fn native_file_calls_without_context_or_path_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("app-data/vibe-home");
    let project = root.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let protected = root.path().join("app-data/synthetic.txt");
    std::fs::write(&protected, "synthetic").unwrap();
    std::fs::hard_link(&protected, project.join("alias.txt")).unwrap();
    for (name, cwd, vibe_home, input) in [
        (
            "missing cwd",
            None,
            Some(home.as_path()),
            json!({"filePath":"alias.txt"}),
        ),
        (
            "missing VIBE_HOME",
            Some(project.as_path()),
            None,
            json!({"filePath":"alias.txt"}),
        ),
        (
            "empty path",
            Some(project.as_path()),
            Some(home.as_path()),
            json!({"filePath":""}),
        ),
        (
            "missing input",
            Some(project.as_path()),
            Some(home.as_path()),
            Value::Null,
        ),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_vw-agent-tools"));
        command
            .args(["guard", "folder", "--endpoint-file"])
            .arg(root.path().join("absent-endpoint.json"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        if let Some(home) = vibe_home {
            command.env("VIBE_HOME", home);
        } else {
            command.env_remove("VIBE_HOME");
        }
        let mut child = command.spawn().unwrap();
        let mut payload = json!({"tool_name":"file_system.read_file", "tool_input":input});
        if let Some(cwd) = cwd {
            payload["cwd"] = json!(cwd);
        }
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{name}: {output:?}");
        let answer: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        assert_eq!(answer["decision"], "deny", "{name}: {answer}");
    }
}

#[test]
fn another_servers_read_file_name_does_not_gain_a_native_requirement() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("app-data/vibe-home");
    let project = root.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_vw-agent-tools"))
        .args(["guard", "folder", "--endpoint-file"])
        .arg(root.path().join("absent-endpoint.json"))
        .env("VIBE_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let payload = json!({"cwd":project,"tool_name":"mcp_other.read_file","tool_input":{}});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}

#[test]
fn a_new_native_file_stays_on_the_path_rule() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("app-data/vibe-home");
    let project = root.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_vw-agent-tools"))
        .args(["guard", "folder", "--endpoint-file"])
        .arg(root.path().join("absent-endpoint.json"))
        .env("VIBE_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let payload = json!({"cwd":project,"tool_name":"file_system.write_file",
        "tool_input":{"path":"new.txt","content":"ordinary"}});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}

#[cfg(windows)]
#[test]
fn an_exclusively_held_safe_alias_reaches_the_core() {
    use std::os::windows::fs::OpenOptionsExt;

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("app-data/vibe-home");
    let project = root.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let safe = project.join("safe.txt");
    std::fs::write(&safe, "ordinary").unwrap();
    std::fs::hard_link(&safe, project.join("alias.txt")).unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&safe)
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = root.path().join("hook-endpoint.json");
    std::fs::write(
        &endpoint,
        json!({"endpoint":listener.local_addr().unwrap().to_string(),"credential":"synthetic-hook"}).to_string(),
    )
    .unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut line = String::new();
                    BufReader::new(&stream).read_line(&mut line).unwrap();
                    let request: Value = serde_json::from_str(&line).unwrap();
                    if request["tool"] == "guard_refused" {
                        stream.write_all(b"{\"ok\":{}}\n").unwrap();
                        continue;
                    }
                    assert_eq!(request["tool"], "native_file_identity");
                    stream.write_all(b"{\"ok\":{\"allowed\":true}}\n").unwrap();
                    return true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        }
        false
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_vw-agent-tools"))
        .args(["guard", "folder", "--endpoint-file"])
        .arg(&endpoint)
        .env("VIBE_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let payload = json!({"session_id":"test","cwd":project,"tool_name":"file_system.read_file",
        "tool_call_id":"exclusive-safe","tool_input":{"path":"alias.txt"}});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let contacted = server.join().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(contacted, "the core was not consulted");
    assert!(output.stdout.is_empty(), "{output:?}");
    drop(held);
}
