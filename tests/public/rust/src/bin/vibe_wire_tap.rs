//! A recording tap between the core and the real Vibe, used as the supervisor's launcher prefix in tests:
//! `vibe_wire_tap [--methods F] [--channels F] [--agent-config F] [--params F] [--context F] -- <server argv...>`.
//! It starts the server, copies every byte of stdin to it and every byte of its stdout back unchanged, and appends
//! what the core sent to the given files, in the formats the old fake server wrote (so a test that proves "this call
//! never reached Vibe" reads the same file whatever the backend):
//! - `--methods`: one request method per line (`initialize` left out);
//! - `--channels`: `<channel> <method> <sessionId|-> <agent|->` per request (channel 0 = not multiplexed);
//! - `--agent-config`: `{"method", "agentConfig", "planProfile"}` per request with an `agentConfig` (`planProfile`:
//!   `VIBE_HOME/agents/vw-plan.toml` at that moment);
//! - `--params`: `{"method", "params"}` per request, exactly as it arrived;
//! - `--responses`: raw server output bytes, for correlating real assigned IDs after a local failure;
//! - `--context`: `{"sessionId", "context": [texts], "user": [texts]}` per `session/turn/enqueue` with context entries.
//!
//! It never answers or rewrites requests. With --test-root it refuses invisible folders before forwarding.
//! stdin EOF is passed on;
//! the tap exits with the server's code. Linux/macOS: the server is in the tap's process group, so the supervisor's
//! group kill reaches it; with bwrap `--die-with-parent` the sandbox also ends with the tap.
//!
//! Depends on: serde_json (std otherwise). Called by: `common/real_vibe.rs` (`tapped`), `shell_host`.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use vw_public_tests::test_paths;

#[derive(Default)]
struct Logs {
    methods: Option<PathBuf>,
    channels: Option<PathBuf>,
    agent_config: Option<PathBuf>,
    params: Option<PathBuf>,
    context: Option<PathBuf>,
    responses: Option<PathBuf>,
    launch: Option<PathBuf>,
    requests: Option<PathBuf>,
    plugin_roots: Option<PathBuf>,
}

/// Check folder-bearing requests before forwarding them, including multiplex channel frames.
fn check_folders(root: &std::path::Path, value: &Value) -> Result<(), String> {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(
                    key.as_str(),
                    "cwd" | "workdir" | "sessionCwd" | "repoLocalPaths" | "workspaceRoots"
                ) {
                    let folders = value.as_str().into_iter().chain(
                        value
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str),
                    );
                    for folder in folders {
                        test_paths::require_inside(root, std::path::Path::new(folder))?;
                    }
                }
                check_folders(root, value)?;
            }
        }
        Value::Array(items) => {
            for value in items {
                check_folders(root, value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Observe the directories visible to the real process at its loading boundary, without parsing manifests.
fn record_plugin_roots(logs: &Logs, method: &str) {
    if logs.plugin_roots.is_none()
        || !matches!(
            method,
            "server/start"
                | "session/start"
                | "session/continue"
                | "session/resume"
                | "session/fork"
                | "plugin/reload"
                | "config/read"
        )
    {
        return;
    }
    let root = std::env::var_os("VIBE_HOME").map(|home| PathBuf::from(home).join("plugins"));
    let mut folders: Vec<_> = root
        .and_then(|root| std::fs::read_dir(root).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|ty| ty.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    folders.sort();
    append(
        &logs.plugin_roots,
        &json!({"method":method,"folders":folders}).to_string(),
    );
}

fn append(path: &Option<PathBuf>, line: &str) {
    let Some(path) = path else { return };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

fn texts(entries: &Value, role: &str) -> Vec<Value> {
    entries
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["role"] == role)
        .flat_map(|e| e["content"].as_array().cloned().unwrap_or_default())
        .filter_map(|b| b.get("text").cloned())
        .collect()
}

/// Records one line the core sent (a JSON-RPC frame, or a multiplex frame `{"channelId", "message"}`).
fn record(logs: &Logs, line: &str) {
    append(&logs.requests, line);
    let Ok(frame) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let (channel, message) = match frame.get("channelId") {
        Some(id) => (
            id.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| id.to_string()),
            frame.get("message").cloned().unwrap_or(Value::Null),
        ),
        None => ("0".to_owned(), frame),
    };
    let (Some(method), Some(_)) = (message["method"].as_str(), message.get("id")) else {
        return; // a notification or an answer to Vibe's own request
    };
    let params = message.get("params").cloned().unwrap_or(json!({}));
    record_plugin_roots(logs, method);
    if method != "initialize" {
        append(&logs.methods, method);
    }
    let sid = params["sessionId"]
        .as_str()
        .or_else(|| params["sourceSessionId"].as_str())
        .unwrap_or("-");
    let agent = params
        .pointer("/agentConfig/agent")
        .or_else(|| params.get("agentName"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    append(&logs.channels, &format!("{channel} {method} {sid} {agent}"));
    if let Some(config) = params.get("agentConfig") {
        let plan = std::env::var_os("VIBE_HOME")
            .map(|home| PathBuf::from(home).join("agents/vw-plan.toml"))
            .and_then(|file| std::fs::read_to_string(file).ok());
        append(
            &logs.agent_config,
            &json!({"method": method, "agentConfig": config, "planProfile": plan}).to_string(),
        );
    }
    append(
        &logs.params,
        &json!({"method": method, "params": params}).to_string(),
    );
    if method == "session/turn/enqueue" {
        let context = texts(&params["entries"], "context");
        if !context.is_empty() {
            append(
                &logs.context,
                &json!({"sessionId": sid, "context": context, "user": texts(&params["entries"], "user")})
                    .to_string(),
            );
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "--print-vibe-runtime")
    {
        let runtime = if let Some(server) = args.get(1) {
            vw_public_tests::runtime::checked(std::path::Path::new(server))
        } else {
            vw_public_tests::runtime::selected()
        };
        match runtime {
            Ok(runtime) => println!("{}", serde_json::to_string(&runtime).unwrap()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args == ["--print-test-root"] {
        println!("{}", test_paths::root().display());
        return;
    }
    let Some(split) = args.iter().position(|a| a == "--") else {
        eprintln!("vibe_wire_tap: usage: vibe_wire_tap [--methods F ...] -- <server argv>");
        std::process::exit(2);
    };
    let mut logs = Logs::default();
    let mut test_root = None;
    let mut server_flags = Vec::new();
    let mut it = args[..split].iter();
    while let Some(flag) = it.next() {
        // When used as VW_DEV_SERVER_BIN the product supervisor puts its own mode flags first.
        if flag == "--multiplex" {
            server_flags.push(flag.clone());
            continue;
        }
        let value = it.next().map(PathBuf::from);
        match flag.as_str() {
            "--test-root" => test_root = value,
            "--methods" => logs.methods = value,
            "--channels" => logs.channels = value,
            "--agent-config" => logs.agent_config = value,
            "--params" => logs.params = value,
            "--context" => logs.context = value,
            "--responses" => logs.responses = value,
            "--launch" => logs.launch = value,
            "--requests" => logs.requests = value,
            "--plugin-roots" => logs.plugin_roots = value,
            other => {
                eprintln!("vibe_wire_tap: unknown flag {other}");
                std::process::exit(2);
            }
        }
    }
    // A recording tap can wrap a sandbox guard. Enforce its root at the first observation boundary too.
    if test_root.is_none() {
        test_root = args[split + 1..]
            .windows(2)
            .find(|pair| pair[0] == "--test-root")
            .map(|pair| PathBuf::from(&pair[1]));
    }
    if let Some(root) = &test_root {
        let mut folders = vec![std::env::current_dir().unwrap()];
        folders.extend(
            ["HOME", "VIBE_HOME"]
                .iter()
                .filter_map(std::env::var_os)
                .map(PathBuf::from),
        );
        for folder in folders {
            if let Err(error) = test_paths::require_inside(root, &folder) {
                eprintln!("{error}");
                std::process::exit(2);
            }
        }
    }
    let server = &args[split + 1..];
    let Some((program, rest)) = server.split_first() else {
        eprintln!("vibe_wire_tap: no server command after --");
        std::process::exit(2);
    };
    record_plugin_roots(&logs, "server/start");
    let mut child = Command::new(program)
        .args(rest)
        .args(server_flags)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("vibe_wire_tap: cannot start {program}: {e}");
            std::process::exit(2);
        });
    if let Some(path) = &logs.launch {
        std::fs::write(
            path,
            json!({"program":program,"argv":rest,"pid":child.id()}).to_string(),
        )
        .unwrap();
    }
    let mut to_server = child.stdin.take().unwrap();
    let mut from_server = child.stdout.take().unwrap();
    let responses = logs.responses.clone();
    // Server -> core: a plain byte copy (Vibe's frames are passed on exactly as written).
    let out = std::thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut captured = responses.and_then(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
        });
        let mut buf = [0u8; 64 * 1024];
        loop {
            match from_server.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Some(file) = captured.as_mut() {
                        let _ = file.write_all(&buf[..n]);
                    }
                    let mut output = stdout.lock();
                    if output.write_all(&buf[..n]).is_err() || output.flush().is_err() {
                        break;
                    }
                }
            }
        }
    });
    // Reading the core must not keep stdout open after the guarded server dies. The main thread watches
    // both input and child exit; a nested guard's refusal therefore reaches the caller immediately.
    let (input, received) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = BufReader::new(stdin.lock());
        loop {
            let mut line = Vec::new();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if input.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = out.join();
                std::process::exit(status.code().unwrap_or(1));
            }
            Err(error) => {
                eprintln!("vibe_wire_tap: cannot wait for {program}: {error}");
                std::process::exit(2);
            }
            Ok(None) => {}
        }
        let line = match received.recv_timeout(std::time::Duration::from_millis(10)) {
            Ok(line) => line,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if let Some(root) = &test_root
            && let Ok(frame) = serde_json::from_slice::<Value>(&line)
            && let Err(error) = check_folders(root, &frame)
        {
            eprintln!("{error}");
            let _ = child.kill();
            let _ = child.wait();
            std::process::exit(2);
        }
        record(&logs, String::from_utf8_lossy(&line).trim_end());
        if to_server.write_all(&line).is_err() || to_server.flush().is_err() {
            break;
        }
    }
    drop(to_server); // EOF to the server, as the core's own close would be
    let status = child.wait().map_or(1, |s| s.code().unwrap_or(1));
    let _ = out.join();
    std::process::exit(status);
}
