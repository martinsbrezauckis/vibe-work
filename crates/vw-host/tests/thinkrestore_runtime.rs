//! Real Vibe 3.1.4 to 3.1.5 chat choice restoration through the app core.
//! Uses disposable homes and completed turns against a deterministic loopback provider.
#![allow(clippy::unwrap_used)]

use serde_json::{Value, json};
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vw_core_api::{Command, CommandResult, Core, CoreConfig, Origin};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

const FAKE_PROVIDER: &str = r#"
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        self.reply(b'{"object":"list","data":[{"id":"zai-glm-5-3-flash","object":"model"}]}', 'application/json')

    def do_POST(self):
        length = int(self.headers.get('Content-Length', '0'))
        request = json.loads(self.rfile.read(length))
        if request.get('stream'):
            chunks = [
                {'id':'chatcmpl-local','object':'chat.completion.chunk','created':0,
                 'model':'zai-glm-5-3-flash','choices':[{'index':0,
                 'delta':{'role':'assistant','content':'done'},'finish_reason':None}]},
                {'id':'chatcmpl-local','object':'chat.completion.chunk','created':0,
                 'model':'zai-glm-5-3-flash','choices':[{'index':0,
                 'delta':{},'finish_reason':'stop'}]},
                {'id':'chatcmpl-local','object':'chat.completion.chunk','created':0,
                 'model':'zai-glm-5-3-flash','choices':[],
                 'usage':{'prompt_tokens':1,'completion_tokens':1,'total_tokens':2}},
            ]
            body = ''.join('data: '+json.dumps(chunk)+'\n\n' for chunk in chunks).encode() + b'data: [DONE]\n\n'
            self.reply(body, 'text/event-stream')
        else:
            body = json.dumps({'id':'chatcmpl-local','object':'chat.completion','created':0,
                'model':'zai-glm-5-3-flash','choices':[{'index':0,
                'message':{'role':'assistant','content':'done'},'finish_reason':'stop'}],
                'usage':{'prompt_tokens':1,'completion_tokens':1,'total_tokens':2}}).encode()
            self.reply(body, 'application/json')

    def reply(self, body, content_type):
        self.send_response(200)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
print(server.server_port, flush=True)
server.serve_forever()
"#;

struct FakeProvider(Child);

impl Drop for FakeProvider {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fake_provider(root: &Path) -> (FakeProvider, u16) {
    let script = root.join("fake_provider.py");
    std::fs::write(&script, FAKE_PROVIDER).unwrap();
    let child = ProcessCommand::new("/usr/bin/python3")
        .arg("-I")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut provider = FakeProvider(child);
    let mut output = BufReader::new(provider.0.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    let port = line.trim().parse::<u16>().unwrap();
    (provider, port)
}

struct NoPrompt;

impl Confirmer for NoPrompt {
    async fn confirm(&self, _: &ConfirmationRequest) -> Confirmation {
        panic!("offline selection and reopen must not ask for confirmation");
    }
}

fn app_server(name: &str, expected: &str) -> PathBuf {
    let python = PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("set {name} to the pinned Vibe Python")),
    );
    let version = ProcessCommand::new(&python)
        .args([
            "-I",
            "-c",
            "import importlib.metadata; print(importlib.metadata.version('mistral-vibe'))",
        ])
        .output()
        .unwrap();
    assert!(
        version.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&version.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&version.stdout).trim(), expected);
    eprintln!("THINKRESTORE_RUNTIME {expected} {}", python.display());
    python.with_file_name("vibe-app-server")
}

async fn start(root: &Path, server: PathBuf) -> Core {
    let work = root.join("work");
    let mut config = vw_supervisor::SupervisorConfig::new(server, root.join("vibe-home"), work);
    config.home = Some(root.join("home"));
    config.parent_env = Some(vec![
        (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
        (OsString::from("LANG"), OsString::from("C.UTF-8")),
    ]);
    config.extra_env.push((
        "PYTHON_KEYRING_BACKEND".into(),
        "keyring.backends.null.Keyring".into(),
    ));
    let mut core_config = CoreConfig::default();
    core_config.instructions.cli_home = Some(root.join("cli"));
    core_config.sessions_dir = Some(root.join("sessions"));
    Box::pin(Core::start(
        config,
        Arc::new(vw_supervisor::NoSecret),
        NoPrompt,
        core_config,
    ))
    .await
    .unwrap()
}

async fn call(core: &Core, method: &str, params: Value) -> Value {
    let result = core
        .execute(
            Origin::Ui,
            Command::Call {
                method: method.into(),
                params,
                confirmation_token: None,
            },
        )
        .await
        .unwrap();
    let CommandResult::Reply { result, .. } = result else {
        panic!("expected {method} reply")
    };
    result
}

async fn new_chat(core: &Core, work: &Path, policy: &str) -> String {
    let agent_config = if policy == "auto-approve" {
        json!({"cwd":work,"trustWorkspace":true})
    } else {
        json!({"cwd":work,"approvalPolicy":policy,"trustWorkspace":true})
    };
    let result = core
        .execute(
            Origin::Ui,
            Command::StartSession {
                agent_config: Some(agent_config),
            },
        )
        .await
        .unwrap();
    let CommandResult::Session { snapshot, .. } = result else {
        panic!("expected started session")
    };
    snapshot.session_id
}

async fn pick_low(core: &Core, id: &str) -> Value {
    let chosen = core
        .execute(
            Origin::Ui,
            Command::PickModel {
                model_name: None,
                model_display_name: None,
                session_id: id.into(),
                model_alias: Some("zai-glm-5-3-flash".into()),
                reasoning_effort: Some("low".into()),
            },
        )
        .await
        .unwrap();
    let CommandResult::Reply { result, .. } = chosen else {
        panic!("expected model pick reply")
    };
    assert_ne!(result["modelChoiceFallback"], true, "{result}");
    result
}

fn selection(result: &Value) -> (&str, &str) {
    let model = &result["runtime"]["config"]["activeModel"];
    (
        model["alias"].as_str().unwrap(),
        model["thinking"].as_str().unwrap(),
    )
}

async fn pin_high_turn(core: &Core, id: &str) {
    let before = call(core, "runtime/read", json!({"sessionId":id})).await;
    assert_eq!(selection(&before), ("zai-glm-5-3-flash", "high"));
    let started = core
        .execute(
            Origin::Ui,
            Command::StartTurn {
                session_id: id.into(),
                message: vec![json!({"type":"text","text":"record this chat"})],
                client_user_message_id: None,
            },
        )
        .await
        .unwrap();
    let CommandResult::Reply { result, .. } = started else {
        panic!("expected turn reply")
    };
    let turn_id = result["turn"]["id"].as_str().unwrap().to_owned();
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let state = call(core, "session/read", json!({"sessionId":id})).await;
        let status = state["state"]["turns"].as_array().and_then(|turns| {
            turns
                .iter()
                .find(|turn| turn["id"] == turn_id)
                .and_then(|turn| turn["status"].as_str())
        });
        if status == Some("completed") {
            eprintln!("THINKRESTORE_314_TURN completed id={id} high");
            return;
        }
        assert!(
            !matches!(status, Some("failed" | "interrupted")),
            "high turn failed: {state}"
        );
        assert!(Instant::now() < until, "high turn did not settle: {state}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
#[ignore = "writes a 3.1.4 fixture with the published 0.1.18 Core"]
async fn write_314_session_fixture() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.1.18");
    let root = PathBuf::from(
        std::env::var_os("THINKRESTORE_FIXTURE_ROOT")
            .unwrap_or_else(|| panic!("set THINKRESTORE_FIXTURE_ROOT")),
    );
    let policy = std::env::var("THINKRESTORE_FIXTURE_POLICY")
        .unwrap_or_else(|_| panic!("set THINKRESTORE_FIXTURE_POLICY"));
    let server = app_server("VIBE_314_PYTHON", "3.1.4");
    let work = root.join("work");
    let old = start(&root, server).await;
    old.register_picked_folder(&work).unwrap();
    let picked = new_chat(&old, &work, &policy).await;
    let untouched = new_chat(&old, &work, &policy).await;
    pin_high_turn(&old, &picked).await;
    pin_high_turn(&old, &untouched).await;
    let result = pick_low(&old, &picked).await;
    assert_eq!(selection(&result), ("zai-glm-5-3-flash", "low"));
    assert_eq!(result["runtime"]["activeApprovalPolicy"]["name"], policy);
    let choices: Value =
        serde_json::from_slice(&std::fs::read(root.join("sessions/model-choices.json")).unwrap())
            .unwrap();
    assert_eq!(choices["chats"][&picked]["pending"]["effort"], "low");
    let live = call(&old, "session/read", json!({"sessionId":picked})).await;
    let live_effort = live["state"]["session"]["reasoningEffort"].as_str();
    std::fs::write(
        root.join("fixture.json"),
        serde_json::to_vec(&json!({"picked":picked,"untouched":untouched,
            "liveEffortAfterPick":live_effort,
            "appVersion":env!("CARGO_PKG_VERSION")}))
        .unwrap(),
    )
    .unwrap();
    eprintln!(
        "THINKRESTORE_314_PICK policy={policy} pending=low runtime=low live_after={live_effort:?}"
    );
    old.shutdown().await;
}

#[tokio::test]
#[ignore = "requires the real Vibe 3.1.4 and 3.1.5 runtimes"]
async fn a_314_chat_pick_keeps_low_thinking_after_315_migration_and_reopen() {
    let new_server = app_server("VIBE_PYTHON", "3.1.5");
    let old_test = PathBuf::from(
        std::env::var_os("THINKRESTORE_314_TEST_BIN")
            .unwrap_or_else(|| panic!("set THINKRESTORE_314_TEST_BIN to the 0.1.18 test binary")),
    );
    for policy in ["ask", "accept-edits", "auto-approve"] {
        run_case(&old_test, &new_server, policy).await;
    }
}

async fn run_case(old_test: &Path, new_server: &Path, policy: &str) {
    let root = tempfile::tempdir().unwrap();
    // A local stdlib server completes the old turns with deterministic text.
    let (_provider, provider_port) = fake_provider(root.path());
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    std::fs::create_dir_all(root.path().join("vibe-home")).unwrap();
    std::fs::write(
        root.path().join("vibe-home/config.toml"),
        format!(
            "enable_telemetry = false\nenable_otel = false\nenable_connectors = false\n\
         experimental_enable_model_catalog = false\nactive_model = 'zai-glm-5-3-flash'\n\
         default_approval_policy = '{policy}'\n\
         [[models]]\nalias = 'zai-glm-5-3-flash'\nname = 'zai-glm-5-3-flash'\n\
         provider = 'llamacpp'\nthinking = 'high'\nthinking_levels = ['low', 'high']\n\
         [[providers]]\nname = 'llamacpp'\napi_base = 'http://127.0.0.1:{provider_port}/v1'\n\
         backend = 'generic'\n"
        ),
    )
    .unwrap();

    let fixture = ProcessCommand::new(old_test)
        .args([
            "--ignored",
            "--exact",
            "write_314_session_fixture",
            "--nocapture",
        ])
        .env("THINKRESTORE_FIXTURE_ROOT", root.path())
        .env("THINKRESTORE_FIXTURE_POLICY", policy)
        .output()
        .unwrap();
    eprintln!("{}", String::from_utf8_lossy(&fixture.stdout));
    eprintln!("{}", String::from_utf8_lossy(&fixture.stderr));
    assert!(
        fixture.status.success(),
        "published 0.1.18 fixture failed for {policy}"
    );
    let ids: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("fixture.json")).unwrap()).unwrap();
    let picked = ids["picked"].as_str().unwrap().to_owned();
    let untouched = ids["untouched"].as_str().unwrap().to_owned();
    assert_eq!(ids["appVersion"], "0.1.18");

    let migrated = vw_host::experiment_pins::ensure(&root.path().join("vibe-home")).unwrap();
    assert!(migrated.skipped.is_none(), "{migrated:?}");
    let new = start(root.path(), new_server.to_path_buf()).await;
    new.register_picked_folder(&work).unwrap();
    let stored = call(&new, "session/read", json!({"sessionId":picked})).await;
    eprintln!(
        "THINKRESTORE_315_STORED policy={policy} effort={:?}",
        stored["state"]["session"]["reasoningEffort"]
    );
    let default = call(&new, "config/read", json!({})).await;
    assert_eq!(
        default["config"]["activeModel"]["thinking"], "high",
        "{default}"
    );
    eprintln!("THINKRESTORE_315_DEFAULT policy={policy} high");
    // The app reads a detached chat before attaching it. Its composer must
    // show the accepted pick even while Vibe's stored level is stale or unset.
    let detached = call(&new, "runtime/read", json!({"sessionId":picked})).await;
    assert_eq!(detached["ready"], false, "{detached}");
    assert_eq!(
        selection(&detached),
        ("zai-glm-5-3-flash", "low"),
        "detached composer ignored the accepted pick: {detached}"
    );
    eprintln!(
        "THINKRESTORE_315_DETACHED policy={policy} observed={:?}",
        selection(&detached)
    );
    for id in [&picked, &untouched] {
        new.execute(
            Origin::Ui,
            Command::ResumeSession {
                session_id: id.clone(),
                agent_config: None,
            },
        )
        .await
        .unwrap();
    }
    let reopened = call(&new, "runtime/read", json!({"sessionId":picked})).await;
    let other = call(&new, "runtime/read", json!({"sessionId":untouched})).await;
    assert_eq!(reopened["runtime"]["activeApprovalPolicy"]["name"], policy);
    assert_eq!(other["runtime"]["activeApprovalPolicy"]["name"], policy);
    assert_eq!(
        selection(&reopened),
        ("zai-glm-5-3-flash", "low"),
        "{reopened}"
    );
    assert_eq!(selection(&other), ("zai-glm-5-3-flash", "high"), "{other}");
    eprintln!("THINKRESTORE_315_REOPEN policy={policy} picked=low untouched=high");
    new.shutdown().await;
}
