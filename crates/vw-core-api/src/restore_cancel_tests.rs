//! Cancellation at the real resume/restore boundary over a deterministic stdio mux peer.
//! Uses the supervisor, channel registry and gate; no browser, model or provider login is used.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::{Core, CoreConfig, ErrorCode, Origin};

const SERVER: &str = r"#!/usr/bin/env python3
import json, os, pathlib, sys, threading, time
home = pathlib.Path(os.environ['VIBE_HOME'])
output = threading.Lock()
def emit(frame):
    with output:
        print(json.dumps(frame), flush=True)
def answer(frame, message, result):
    frame = dict(frame)
    frame['message'] = {'jsonrpc':'2.0', 'id':message['id'], 'result':result}
    emit(frame)
def held(frame, message, result, marker):
    (home / marker).touch()
    while not (home / 'release').exists():
        time.sleep(.005)
    answer(frame, message, result)
for line in sys.stdin:
    frame = json.loads(line)
    message = frame['message']
    if message is None:
        emit(frame)
        continue
    if 'id' not in message:
        continue
    method = message['method']
    params = message.get('params', {})
    sid = params.get('sessionId', 'chat')
    with (home / 'calls').open('a') as log:
        log.write(json.dumps(message) + '\n')
    if method == 'initialize':
        result = {'serverInfo':{'name':'restore-cancel-test','version':'0'}}
    elif method == 'session/list':
        result = {'items':[], 'nextCursor':None}
    elif method in ('session/read', 'session/resume'):
        if method == 'session/resume' and sid == 'import':
            sid = 'imported'
        result = {'state':{'session':{'id':sid,'cwd':str(home.parent),
            'status':{'type':'idle'}},'turns':[], 'history':[], 'eventId':0}}
    elif method == 'session/approvalPolicy/update':
        result = {'runtime':{'activeApprovalPolicy':{'name':params['approvalPolicy']}}}
        if (home / 'update-result.json').exists():
            result = json.loads((home / 'update-result.json').read_text())
    elif method == 'loops/list':
        result = {'loops':[]}
    elif method == 'config/read':
        result = {'config':{'models':[]}}
    else:
        result = {}
    marker = None
    if method == 'session/approvalPolicy/update' and sid != 'second':
        marker = 'approval-held'
    if method == 'session/read' and (home / 'hold-read').exists():
        marker = 'read-held'
    if marker and not (home / 'release').exists():
        threading.Thread(target=held,args=(frame,message,result,marker),daemon=True).start()
    else:
        answer(frame,message,result)
";

struct Rig {
    core: Arc<Core>,
    root: PathBuf,
    home: PathBuf,
}

// Fixture setup failures must fail the test at the point where the fixture is prepared.
#[allow(clippy::unwrap_used)]
impl Rig {
    async fn new(name: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("vw-restore-cancel-{name}-{}", std::process::id()));
        let home = root.join("vibe");
        std::fs::create_dir_all(&home).unwrap();
        let program = root.join("server.py");
        std::fs::write(&program, SERVER).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let core = Box::pin(Core::start(
            vw_supervisor::SupervisorConfig::new(program, home.clone(), root.clone()),
            Arc::new(vw_supervisor::NoSecret),
            crate::refresh_notice_tests::AlwaysAllow,
            CoreConfig {
                reopen_timeout: Duration::from_secs(2),
                instructions: crate::InstructionsConfig {
                    cli_home: Some(root.join("cli")),
                },
                ..CoreConfig::default()
            },
        ))
        .await
        .unwrap();
        Self {
            core: Arc::new(core),
            root,
            home,
        }
    }

    async fn reached(&self, marker: &str) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !self.home.join(marker).exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    fn release(&self) {
        std::fs::write(self.home.join("release"), "").unwrap();
    }

    async fn send(&self, id: &str) -> Result<vw_gate::Reply, crate::CoreError> {
        self.core
            .forward(
                Origin::Ui,
                "turn/start",
                json!({
                    "sessionId":id, "message":[{"type":"text","text":"Continue"}]
                }),
                None,
            )
            .await
    }

    async fn finish(self) {
        self.release();
        self.core.shutdown().await;
        std::fs::remove_dir_all(self.root).unwrap();
    }
}

#[tokio::test]
async fn bounded_open_removes_the_unverified_resumed_channel() {
    let rig = Rig::new("timeout").await;
    let core = rig.core.clone();
    let opening = tokio::spawn(async move {
        core.bounded_open("chat", core.attach_session(Origin::Ui, "chat"))
            .await
    });
    rig.reached("approval-held").await;
    let key = rig.core.shared.channels.key_of("chat").unwrap();
    assert!(rig.core.shared.channels.is_restoring("chat"));
    assert_eq!(
        opening.await.unwrap().unwrap_err().code,
        ErrorCode::SessionStuck
    );
    assert_eq!(
        rig.core.shared.channels.key_of("chat"),
        None,
        "unverified slot survived timeout"
    );
    assert!(!rig.core.shared.channels.has_channel("chat"));
    assert_eq!(
        rig.send("chat").await.unwrap_err().code,
        ErrorCode::SessionStuck
    );
    rig.release();
    rig.core.retry_reopen(Origin::Ui, "chat").await.unwrap();
    rig.send("chat").await.unwrap();
    assert_ne!(rig.core.shared.channels.key_of("chat"), Some(key));
    assert!(rig.core.shared.channels.has_channel("chat"));
    rig.finish().await;
}

#[tokio::test]
async fn abort_removes_only_its_channel_and_a_second_chat_keeps_working() {
    for mode in ["ask", "accept-edits", "auto-approve"] {
        let rig = Rig::new(&format!("abort-{mode}")).await;
        rig.core
            .shared
            .access
            .observe_runtime("chat", &json!({"activeApprovalPolicy":{"name":mode}}));
        let core = rig.core.clone();
        let opening = tokio::spawn(async move { core.attach_session(Origin::Ui, "chat").await });
        rig.reached("approval-held").await;
        rig.core.attach_session(Origin::Ui, "second").await.unwrap();
        assert!(rig.core.shared.channels.has_channel("second"));
        opening.abort();
        assert!(opening.await.unwrap_err().is_cancelled());
        assert_eq!(
            rig.core.shared.channels.key_of("chat"),
            None,
            "unverified slot survived abort in {mode}"
        );
        assert!(rig.core.shared.channels.has_channel("second"));
        rig.release();
        rig.send("chat").await.unwrap();
        assert!(rig.core.shared.channels.has_channel("chat"));
        assert_eq!(
            rig.core.shared.access.remembered("chat").as_deref(),
            Some(mode)
        );
        let calls: Vec<serde_json::Value> = std::fs::read_to_string(rig.home.join("calls"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|call: &serde_json::Value| call["params"]["sessionId"] == "chat")
            .collect();
        let methods: Vec<_> = calls
            .iter()
            .filter_map(|call| call["method"].as_str())
            .collect();
        assert_eq!(
            methods
                .iter()
                .filter(|method| **method == "session/resume")
                .count(),
            2
        );
        let verified = methods
            .iter()
            .rposition(|method| *method == "session/approvalPolicy/update")
            .unwrap();
        let sent = methods
            .iter()
            .position(|method| *method == "turn/start")
            .unwrap();
        assert!(
            verified < sent,
            "the new send must follow a fresh verified restore: {methods:?}"
        );
        rig.finish().await;
    }
}

#[tokio::test]
async fn cancellation_does_not_close_a_later_replacement() {
    let rig = Rig::new("replacement").await;
    let core = rig.core.clone();
    let opening = tokio::spawn(async move { core.attach_session(Origin::Ui, "chat").await });
    rig.reached("approval-held").await;
    let cancelled = rig.core.shared.channels.bound("chat").unwrap();
    let methods = Arc::default();
    let peer = crate::refresh_notice_tests::attach_mock_channel(&rig.core, "chat", 900, methods);
    opening.abort();
    assert!(opening.await.unwrap_err().is_cancelled());
    assert_eq!(rig.core.shared.channels.key_of("chat"), Some((0, 900)));
    rig.core
        .shared
        .channels
        .request("loops/list", json!({"sessionId":"chat"}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), cancelled.closed())
        .await
        .expect("the cancelled transport must close even after its slot was replaced");
    rig.finish().await;
    peer.abort();
}

#[tokio::test]
async fn verified_restore_and_already_attached_channel_remain_usable() {
    let rig = Rig::new("verified").await;
    rig.release();
    rig.core.attach_session(Origin::Ui, "chat").await.unwrap();
    let key = rig.core.shared.channels.key_of("chat").unwrap();
    rig.core.attach_session(Origin::Ui, "chat").await.unwrap();
    assert_eq!(rig.core.shared.channels.key_of("chat"), Some(key));
    assert!(rig.core.shared.channels.has_channel("chat"));
    rig.finish().await;
}

#[tokio::test]
async fn abort_while_reading_the_profile_does_not_strand_the_restore_mark() {
    let rig = Rig::new("read-abort").await;
    std::fs::write(rig.home.join("hold-read"), "").unwrap();
    let core = rig.core.clone();
    let reading = tokio::spawn(async move { core.begin_restore("chat", None).await });
    rig.reached("read-held").await;
    reading.abort();
    assert!(reading.await.is_err());
    assert!(!rig.core.shared.channels.is_restoring("chat"));
    rig.finish().await;
}

#[tokio::test]
async fn abort_removes_an_imported_channel_under_its_actual_id() {
    let rig = Rig::new("import").await;
    let core = rig.core.clone();
    let opening = tokio::spawn(async move { core.attach_session(Origin::Ui, "import").await });
    rig.reached("approval-held").await;
    assert!(rig.core.shared.channels.key_of("imported").is_some());
    opening.abort();
    assert!(opening.await.unwrap_err().is_cancelled());
    assert_eq!(rig.core.shared.channels.key_of("imported"), None);
    rig.finish().await;
}

#[tokio::test]
async fn an_imported_binding_is_hidden_before_observation_and_removed_synchronously() {
    let rig = Rig::new("import-before-observe").await;
    let restore = rig.core.begin_restore("import", None).await;
    let (params, token) = restore.resume_request(false).unwrap();
    rig.core
        .shared
        .gate
        .call("session/resume", params, token.as_ref())
        .await
        .unwrap();
    assert!(rig.core.shared.channels.key_of("imported").is_some());
    assert!(
        !rig.core.shared.channels.has_channel("imported"),
        "import was exposed before verification"
    );
    drop(restore);
    assert_eq!(
        rig.core.shared.channels.key_of("imported"),
        None,
        "detachment must not wait for a spawned task"
    );
    rig.finish().await;
}

#[tokio::test]
async fn abort_of_a_forced_resume_invalidates_the_exact_preexisting_channel() {
    let rig = Rig::new("forced-abort").await;
    rig.release();
    rig.core.attach_session(Origin::Ui, "chat").await.unwrap();
    let key = rig.core.shared.channels.key_of("chat").unwrap();
    std::fs::remove_file(rig.home.join("release")).unwrap();
    let core = rig.core.clone();
    let opening = tokio::spawn(async move {
        core.open_session(Origin::Ui, "chat", Duration::from_secs(20))
            .await
    });
    rig.reached("approval-held").await;
    assert_eq!(rig.core.shared.channels.key_of("chat"), Some(key));
    opening.abort();
    assert!(opening.await.unwrap_err().is_cancelled());
    assert_eq!(rig.core.shared.channels.key_of("chat"), None);
    rig.finish().await;
}

#[tokio::test]
async fn abort_before_resume_preserves_an_existing_verified_channel() {
    let rig = Rig::new("attached-read-abort").await;
    rig.release();
    rig.core.attach_session(Origin::Ui, "chat").await.unwrap();
    let key = rig.core.shared.channels.key_of("chat").unwrap();
    std::fs::remove_file(rig.home.join("release")).unwrap();
    std::fs::write(rig.home.join("hold-read"), "").unwrap();
    let core = rig.core.clone();
    let reading = tokio::spawn(async move { core.begin_restore("chat", None).await });
    rig.reached("read-held").await;
    reading.abort();
    assert!(reading.await.is_err());
    assert_eq!(rig.core.shared.channels.key_of("chat"), Some(key));
    assert!(rig.core.shared.channels.has_channel("chat"));
    rig.finish().await;
}

#[tokio::test]
async fn wrong_empty_or_missing_verification_refuses_visibly() {
    for (name, result) in [
        (
            "wrong",
            json!({"runtime":{"activeApprovalPolicy":{"name":"auto-approve"}}}),
        ),
        (
            "empty",
            json!({"runtime":{"activeApprovalPolicy":{"name":""}}}),
        ),
        ("missing", json!({})),
    ] {
        let rig = Rig::new(name).await;
        rig.release();
        std::fs::write(rig.home.join("update-result.json"), result.to_string()).unwrap();
        let error = rig
            .core
            .attach_session(Origin::Ui, "chat")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AccessModeUnverified);
        assert!(error.message.contains("Nothing was sent"));
        assert_eq!(rig.core.shared.channels.key_of("chat"), None);
        rig.finish().await;
    }
}
