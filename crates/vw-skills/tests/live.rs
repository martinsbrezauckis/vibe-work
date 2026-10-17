//! Opt-in stdio smoke test against the tagged Vibe server from data/venv.
//! Uses disposable project/home, no inherited credentials, and rejects network sockets.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};
use vw_skills::{
    SkillsCatalogParams, SkillsClient, SkillsError, SkillsImportParams, SkillsInstalledParams,
    SkillsListParams, SkillsSetEnabledParams, SkillsTransport, TransportError,
};

struct Channel {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    id: u64,
}

struct StdioTransport(Mutex<Channel>);

impl SkillsTransport for StdioTransport {
    async fn request(&self, method: &str, params: Value) -> Result<Value, TransportError> {
        let mut channel = self.0.lock().await;
        channel.id += 1;
        let id = channel.id;
        let mut bytes = serde_json::to_vec(
            &json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}),
        )
        .unwrap();
        bytes.push(b'\n');
        channel.stdin.write_all(&bytes).await.unwrap();
        channel.stdin.flush().await.unwrap();
        loop {
            let mut line = String::new();
            let count = channel.stdout.read_line(&mut line).await.unwrap();
            assert!(count > 0, "Vibe exited before replying to {method}");
            let reply: Value = serde_json::from_str(&line).unwrap();
            if reply.get("id") != Some(&json!(id)) {
                continue;
            }
            return if let Some(error) = reply.get("error") {
                Err(TransportError::Reply(
                    serde_json::from_value(error.clone()).unwrap(),
                ))
            } else {
                Ok(reply["result"].clone())
            };
        }
    }
}

#[tokio::test]
async fn tagged_vibe_without_credentials_or_network() {
    if std::env::var_os("VW_SKILLS_LIVE").as_deref() != Some(std::ffi::OsStr::new("1")) {
        eprintln!("SKIPPED live Vibe test: set VW_SKILLS_LIVE=1");
        return;
    }
    tokio::time::timeout(Duration::from_secs(45), live())
        .await
        .expect("Vibe smoke test timed out");
}

fn launch(project: &Path, home: &Path, vibe_home: &Path) -> StdioTransport {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let mut child = Command::new(repo.join("data/venv/bin/python"))
        .arg("-c")
        .arg(include_str!("support/live_server.py"))
        .arg(repo.join("data/mistral-vibe"))
        .current_dir(project)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("VIBE_HOME", vibe_home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    StdioTransport(Mutex::new(Channel {
        stdin: child.stdin.take().unwrap(),
        stdout: BufReader::new(child.stdout.take().unwrap()),
        _child: child,
        id: 0,
    }))
}

async fn initialize(transport: &StdioTransport, project: &Path) -> String {
    let initialized = transport
        .request(
            "initialize",
            json!({"clientInfo":{"name":"skills-test","version":"0"}}),
        )
        .await
        .unwrap();
    assert_eq!(initialized["serverInfo"]["version"], "2.26.1");
    {
        let mut channel = transport.0.lock().await;
        channel
            .stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n")
            .await
            .unwrap();
        channel.stdin.flush().await.unwrap();
    }
    let started = transport
        .request(
            "session/start",
            json!({"kind":"ephemeral","agentConfig":{"cwd":project}}),
        )
        .await
        .unwrap();
    started["state"]["session"]["id"]
        .as_str()
        .expect("Session id")
        .to_owned()
}

fn workspace(temp: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let home = temp.join("home");
    let vibe_home = home.join(".vibe");
    let project = temp.join("project");
    std::fs::create_dir_all(&vibe_home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        vibe_home.join("config.toml"),
        include_str!("fixtures/live-config.toml"),
    )
    .unwrap();
    let skill_dir = vibe_home.join("skills/live-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: live-skill\ndescription: Disposable live test skill\n---\nRead the code.\n",
    )
    .unwrap();
    (home, vibe_home, project)
}

async fn live() {
    let temp = tempfile::tempdir().unwrap();
    let (home, vibe_home, project) = workspace(temp.path());
    let transport = launch(&project, &home, &vibe_home);
    let session_id = initialize(&transport, &project).await;
    let client = SkillsClient::new(transport);
    let installed = client
        .installed(&SkillsInstalledParams {
            session_id: session_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        installed
            .skills
            .iter()
            .any(|row| row.name == "live-skill" && row.enabled)
    );
    let active = client
        .list(&SkillsListParams {
            session_id: session_id.clone(),
        })
        .await
        .unwrap();
    assert!(active.skills.iter().any(|row| row.name == "live-skill"));
    let catalog = client
        .catalog(&SkillsCatalogParams {
            session_id: session_id.clone(),
        })
        .await
        .unwrap();
    assert!(catalog.loaded && !catalog.authenticated && catalog.skills.is_empty());
    let disabled = client
        .set_enabled(&SkillsSetEnabledParams {
            session_id: session_id.clone(),
            name: "live-skill".into(),
            enabled: false,
        })
        .await
        .unwrap();
    assert!(
        !disabled
            .runtime
            .skills
            .iter()
            .any(|row| row.name == "live-skill")
    );
    let installed = client
        .installed(&SkillsInstalledParams {
            session_id: session_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        installed
            .skills
            .iter()
            .any(|row| row.name == "live-skill" && !row.enabled)
    );
    let error = client
        .import(&SkillsImportParams {
            session_id,
            skill_id: "unused-id".into(),
            version: None,
            alias: None,
            scope: "global".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, SkillsError::InvalidInput(_)));
    assert_eq!(
        error.to_string(),
        "no authenticated Mistral endpoint available"
    );
    eprintln!(
        "Live Vibe v2.26.1 PASSED: installed, list, unauthenticated catalog, disable, import rejection"
    );
}
