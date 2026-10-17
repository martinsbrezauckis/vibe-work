// Shared synthetic server runtime. Included by the public and private fixture entrypoints.
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Process-wide settings and the scripted world shared by every connection.
pub struct Ctx {
    resume_delay: u64,
    silent_initialize: bool,
    grandchild: Option<u32>,
    agent: Option<u32>,
    scripted: Option<Arc<script::Script>>,
}

/// One connection: the whole stdio stream (`key` 0) or one channel of a multiplexed process.
pub struct Conn {
    pub key: u64,
    pub send: script::Send,
}

impl Ctx {
    /// Handles one JSON-RPC message that arrived on `conn`. Returns true when the server ends this
    /// connection afterwards (`session/stop` in script mode: Vibe's channel server closes itself).
    pub fn on_frame(&self, conn: &Conn, frame: &Value) -> bool {
        let send = &conn.send;
        let Some(method) = frame.get("method").and_then(Value::as_str) else {
            if let Some(script) = &self.scripted {
                script.on_response(frame);
            }
            return false;
        };
        let Some(id) = frame.get("id").cloned() else {
            return false;
        };
        let params = frame.get("params").cloned().unwrap_or(json!({}));
        if let Some(script) = &self.scripted {
            script.log_channel(conn.key, method, &params);
        }
        log_agent_config(method, &params);
        if let Some(script) = self.scripted.as_ref().filter(|_| method != "initialize") {
            script.log_method(method);
            if method == "mcp_catalog/login" {
                // the fixture: answered only after the browser step, like Vibe (`connectors.rs`).
                let (script, send, key) = (script.clone(), send.clone(), conn.key);
                std::thread::spawn(move || {
                    let result = script.login_blocking(key, &params);
                    send(reply_frame(&id, result));
                });
                return false;
            }
            if method == "session/resume" && self.resume_delay > 0 {
                // Answered later from a thread: other channels keep being served meanwhile, like
                // Vibe's channels (each its own server) while one harness holds a session lock.
                let (script, send, key, delay) =
                    (script.clone(), send.clone(), conn.key, self.resume_delay);
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(delay));
                    if let Some(result) = script.handle(key, "session/resume", &params) {
                        send(reply_frame(&id, result));
                    }
                });
                return false;
            }
            let mut stop = false;
            if let Some(result) = script.handle(conn.key, method, &params) {
                stop = method == "session/stop" && result.get("__error").is_none();
                send(reply_frame(&id, result));
            }
            return stop;
        }
        let reply = |result: Value| json!({"jsonrpc": "2.0", "id": id, "result": result});
        match method {
            "initialize" if self.silent_initialize => {}
            "initialize" => send(reply(json!({
                "serverInfo": {"name": "fake-vibe", "version": "9.9.9"},
                "echoClientInfo": params["clientInfo"],
            }))),
            "echo" => send(reply(params)),
            "env/dump" => {
                let env: serde_json::Map<String, Value> = std::env::vars()
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect();
                send(reply(json!({"env": env})));
            }
            "self/exe" => send(reply(
                json!({"exe": std::env::current_exe().unwrap(), "cwd": std::env::current_dir().unwrap()}),
            )),
            "pid/info" => send(reply(
                json!({"pid": std::process::id(), "grandchild": self.grandchild, "agent": self.agent, "channel": conn.key}),
            )),
            "session/resume" => {
                let send = send.clone();
                let delay = self.resume_delay;
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(delay));
                    send(
                        json!({"jsonrpc": "2.0", "id": id, "result": {"resumed": params["sessionId"]}}),
                    );
                });
            }
            "crash/now" => {
                if let Some(text) = params["stderr"].as_str() {
                    eprint!("{text}");
                }
                let _ = std::io::stderr().flush();
                std::process::exit(params["code"].as_i64().unwrap_or(1) as i32);
            }
            "callback/result" => {}
            _ => send(
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": "method_not_found", "message": "fake"}}),
            ),
        }
        false
    }
}

/// The answer frame for a script result (`{"__error": code}` becomes an error object).
fn reply_frame(id: &Value, result: Value) -> Value {
    match result.get("__error").and_then(Value::as_str) {
        Some(code) => {
            let mut frame = json!({"jsonrpc": "2.0", "id": id,
            "error": {"code": code, "message": result.get("__message").and_then(Value::as_str)
                .unwrap_or("fake: no such session")}});
            if let Some(data) = result.get("__data") {
                frame["error"]["data"] = data.clone();
            }
            frame
        }
        None => json!({"jsonrpc": "2.0", "id": id, "result": result}),
    }
}

/// `VW_FAKE_LOG_AGENT_CONFIG=<file>` (the fixture): appends `{"method", "agentConfig"}` of every request that
/// carries an `agentConfig`, so a test can see exactly what the core sent (the injected MCP server), plus the
/// `vw-plan` profile text at that moment (the fixture).
/// `--log-agent-config=<file>` (the fixture): the same as `VW_FAKE_LOG_AGENT_CONFIG`, for a real app run, whose server
/// environment is an allowlist (the variable never reaches the fake there).
static AGENT_LOG: std::sync::OnceLock<std::ffi::OsString> = std::sync::OnceLock::new();

fn log_agent_config(method: &str, params: &Value) {
    let (Some(path), Some(config)) = (
        std::env::var_os("VW_FAKE_LOG_AGENT_CONFIG").or_else(|| AGENT_LOG.get().cloned()),
        params.get("agentConfig"),
    ) else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        // the fixture: what `VIBE_HOME/agents/vw-plan.toml` said when this request arrived (proves the core wrote
        // the profile BEFORE the session was opened).
        let plan_profile = std::env::var_os("VIBE_HOME")
            .map(|home| std::path::PathBuf::from(home).join("agents/vw-plan.toml"))
            .and_then(|file| std::fs::read_to_string(file).ok());
        let _ = writeln!(
            file,
            "{}",
            json!({"method": method, "agentConfig": config, "planProfile": plan_profile})
        );
    }
}

fn main() {
    crate::market::record_load("server/start");
    let args: Vec<String> = std::env::args().collect();
    if let Some(path) = args
        .iter()
        .find_map(|a| a.strip_prefix("--log-agent-config="))
    {
        let _ = AGENT_LOG.set(std::ffi::OsString::from(path));
    }
    // `--dump-start <file>` (the fixture, the product launch test): what this process was started with.
    if let Some(path) = arg_value(&args, "--dump-start") {
        let env: serde_json::Map<String, Value> = std::env::vars()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        let dump = json!({"argv": args, "env": env, "exe": std::env::current_exe().unwrap(), "cwd": std::env::current_dir().unwrap()});
        std::fs::write(path, dump.to_string()).unwrap();
    }
    if arg_value(&args, "--mode").as_deref() == Some("sleeper") {
        std::thread::sleep(Duration::from_secs(300));
        return;
    }
    if let Some(path) = arg_value(&args, "--state-file") {
        let count: u32 = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
            + 1;
        std::fs::write(&path, count.to_string()).unwrap();
        let from: u32 =
            arg_value(&args, "--crash-from-start").map_or(u32::MAX, |v| v.parse().unwrap());
        let until: u32 =
            arg_value(&args, "--crash-until-start").map_or(u32::MAX, |v| v.parse().unwrap());
        if count >= from && count <= until {
            eprintln!("fake crash on start {count}");
            std::process::exit(3);
        }
    }
    if args.iter().any(|a| a == "--stderr-key-at-start") {
        let key = std::env::var("MISTRAL_API_KEY").unwrap_or_default();
        eprintln!("Traceback (most recent call last):");
        eprintln!("  File \"fake_agent.py\", line 7, in send");
        eprintln!(
            "RuntimeError: FAKE-TRACEBACK-TEXT request failed with Authorization: Bearer {key}"
        );
    }
    let grandchild = args.iter().any(|a| a == "--spawn-grandchild").then(|| {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--mode", "sleeper"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
            .id()
    });
    let agent = args.iter().any(|a| a == "--spawn-agent").then(|| {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--mode", "sleeper"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0000_0200);
        command.spawn().unwrap().id()
    });
    let multiplex = args.iter().any(|a| a == "--multiplex");
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let raw: script::Send = Arc::new(move |frame: Value| {
        let mut out = out.lock().unwrap();
        let _ = writeln!(out, "{frame}");
        let _ = out.flush();
    });
    // the fixture: `--history-log FILE` records each `session/history/list` request (`history.rs`).
    history::set_log(arg_value(&args, "--history-log"));
    history::set_holders(arg_value(&args, "--lease-holders"));
    let scripted = args.iter().any(|a| a == "--script").then(|| {
        script::Script::new(script::Options {
            log_path: arg_value(&args, "--log-methods"),
            channel_log_path: arg_value(&args, "--log-channels"),
            bypass_profile: arg_value(&args, "--bypass-profile"),
            status_events: args.iter().any(|a| a == "--status-events"),
            #[cfg(feature = "fixture-extensions")]
            long_notification_error: args.iter().any(|a| a == "--long-notification-error"),
            unstored_metadata_miss: args.iter().any(|a| a == "--unstored-metadata-miss"),
            archive_no_notify: args.iter().any(|a| a == "--archive-no-notify"),
            seed_path: arg_value(&args, "--seed"),
            single_attached: args.iter().any(|a| a == "--single-attached"),
            turn_delay_ms: arg_value(&args, "--turn-delay-ms")
                .map_or(0, |v| v.parse().unwrap_or(0)),
            multiplex,
            persist: arg_value(&args, "--persist"),
            agent_fault: arg_value(&args, "--agent-update-fault"),
            stuck_resumes: arg_value(&args, "--stuck-resumes")
                .map_or(0, |v| v.parse().unwrap_or(0)),
            edit_files: args.iter().any(|a| a == "--edit-files"),
            rich_turn: args.iter().any(|a| a == "--rich-turn"),
            #[cfg(feature = "fixture-extensions")]
            extended_turn: private_hook_turn_enabled(&args),
            subagents_rule: args.iter().any(|a| a == "--subagents-rule"),
            subagents_reject: args.iter().any(|a| a == "--subagents-reject"),
            #[cfg(feature = "fixture-extensions")]
            proxy_echo_error: args.iter().any(|a| a == "--proxy-echo-error"),
            delete_faults: arg_value(&args, "--delete-faults")
                .map_or(0, |v| v.parse().unwrap_or(0)),
            context_log_path: arg_value(&args, "--log-context"),
            model_config_path: arg_value(&args, "--model-config"),
            model_log_path: arg_value(&args, "--log-models"),
        })
    });
    let ctx = Ctx {
        resume_delay: arg_value(&args, "--resume-delay-ms").map_or(0, |v| v.parse().unwrap()),
        silent_initialize: args.iter().any(|a| a == "--silent-initialize"),
        grandchild,
        agent,
        scripted,
    };
    if multiplex {
        mux::serve(&ctx, &raw);
    } else {
        let conn = Conn {
            key: 0,
            send: raw.clone(),
        };
        if let Some(script) = &ctx.scripted {
            script.connect(0, raw.clone());
        }
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let _ = ctx.on_frame(&conn, &frame);
        }
    }
    if args.iter().any(|a| a == "--ignore-eof") {
        std::thread::sleep(Duration::from_secs(300));
    }
}
