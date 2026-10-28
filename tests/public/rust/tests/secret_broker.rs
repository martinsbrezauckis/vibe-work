//! Secret broker behavior over a real Vibe session and disposable vault.
//! PREDICT: user consent gates secret use, locked vaults refuse it, and answers withhold values.
#![cfg(unix)]

mod agent_tools;
mod common;
mod procmon;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use agent_tools::{Dirs, call_and_collect as call, credential_of, rig_with};
use common::fake_mistral;
use common::vault::key_service_config;
use serde_json::{Value, json};
use vw_core_api::{Command, CommandResult, Origin};
use vw_gate::{Confirmation, Requester};
use vw_keys::KeyService;
use vw_keys::existing::MemoryVibeKeyring;
use vw_vault::SecretString;

const VALUE: &str = "vwProjectSecretValue0123456789ABCDEFxyz";
const MISTRAL_KEY: &str = "vwFakeMistralKeyBroker0123456789abcdEF";

/// Every log line of this test binary (a global subscriber: the rig runs on several threads).
fn logs() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer: Arc<Mutex<Vec<u8>>> = Arc::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || Sink(writer.clone()))
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
        buffer
    })
}

struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// True when `text` holds `value` in any form the registry searches, or as hex written out here (independent of the
/// registry, so a registry that stops searching hex fails this test).
fn leaked(text: &str, value: &str) -> bool {
    let hex: String = value.bytes().map(|b| format!("{b:02x}")).collect();
    let mut forms = vw_secrets::forms(value);
    forms.push(hex.to_ascii_uppercase().into_bytes());
    forms.push(hex.into_bytes());
    forms
        .iter()
        .any(|f| text.as_bytes().windows(f.len()).any(|w| w == f.as_slice()))
}

async fn secrets(core: &vw_core_api::Core, origin: Origin, request: Value) -> Value {
    let command: Command =
        serde_json::from_value(json!({"type": "secrets", "request": request})).unwrap();
    match core.execute(origin, command).await.unwrap() {
        CommandResult::Secrets { result } => serde_json::to_value(result).unwrap(),
        other => panic!("{other:?}"),
    }
}

/// A 120-character canonical project folder, removed on drop even when the test panics.
/// Deep folders must fit the native consent without changing the requested command or its arguments.
struct DeepProject {
    root: PathBuf,
    folder: PathBuf,
}

impl DeepProject {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let padding = 120 - root.to_string_lossy().chars().count() - 1;
        let folder = root.join("p".repeat(padding));
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(folder.to_string_lossy().chars().count(), 120);
        Self { root, folder }
    }
}

impl Drop for DeepProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn run_args(program: &str, args: &[&str], env: Value) -> Value {
    json!({"program": program, "args": args, "env": env, "purpose": "deploy the test site"})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn the_secret_broker_end_to_end() {
    let pass = format!("test-passphrase-{}", vw_vault::login::new_id());
    let log = logs().clone();
    let fake = fake_mistral::start().await;
    let dirs = Dirs::new("secret-broker");
    vw_changes::prepare_root(&dirs.root.join("changes")).unwrap();
    let kdir = dirs.root.join("keys");
    std::fs::create_dir_all(&kdir).unwrap();
    let service = KeyService::start(key_service_config(
        &kdir,
        &fake.base(),
        Arc::new(MemoryVibeKeyring::default()),
    ))
    .await
    .unwrap();
    service
        .create_vault(SecretString::new(pass.as_str()))
        .await
        .unwrap();
    service
        .store_key(SecretString::new(MISTRAL_KEY))
        .await
        .unwrap();
    let svc = service.clone();
    let rig = rig_with(&dirs, "broker", move |c| c.keys = Some(svc)).await;
    let deep_project = DeepProject::new("vwb");
    let project = deep_project.folder.clone();
    // The program prints the value plainly, as base64 and inside JSON, and writes it to a side file (the falsifier:
    // the output really carried it).
    std::fs::write(
        project.join("printer.sh"),
        "v=$DEPLOY_TOKEN\necho \"$v\"\necho \"$v\"|base64\necho \"{\\\"k\\\":\\\"$v\\\"}\"\n\
         printf %s \"$v\">proof.txt\necho home=${MISTRAL_API_KEY:-none}\n\
         printf %s \"$v\"|od -An -tx1|tr -d ' \\n';echo\n",
    )
    .unwrap();
    std::fs::write(project.join("sleeper.sh"), "sleep 30\n").unwrap();
    let sid = rig.start_session(&project, "ask").await;
    let who = credential_of(&rig.injected(0));

    // The user adds the secret (one-way: the answer has no value).
    let added = secrets(
        &rig.core,
        Origin::Ui,
        json!({"op": "add", "name": "Deploy token", "value": VALUE}),
    )
    .await;
    assert!(!leaked(&added.to_string(), VALUE), "{added}");
    let id = added["secret"]["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("vibe-work-secret-"), "{added}");

    // secret_list: names only; the Mistral key (userOnly) is not there.
    let list = call(&who, "secret_list", json!({})).await.unwrap();
    let text = list.to_string();
    assert!(text.contains("Deploy token"), "{text}");
    assert!(!text.to_lowercase().contains("mistral"), "{text}");
    assert!(
        !leaked(&text, VALUE) && !leaked(&text, MISTRAL_KEY),
        "{text}"
    );

    let env = json!([{"name": "DEPLOY_TOKEN", "secret": "Deploy token"}]);
    let dialogs = |method: &str| {
        rig.confirmer
            .shown()
            .iter()
            .filter(|r| r.method == method)
            .count()
    };

    // Once.
    rig.confirmer.then(Confirmation::Approved);
    let first = call(
        &who,
        "secret_run",
        run_args("sh", &["printer.sh"], env.clone()),
    )
    .await
    .unwrap();
    let first_text = first.to_string();
    assert!(first.get("approval").is_none(), "{first}");
    assert_eq!(first["exitCode"], 0, "{first_text}");
    assert_eq!(
        std::fs::read_to_string(project.join("proof.txt")).unwrap(),
        VALUE,
        "the program got the exact value (falsifier: the output carried it)"
    );
    assert!(
        !leaked(&first_text, VALUE),
        "the answer withholds every form: {first_text}"
    );
    assert!(
        first_text.contains("[secret withheld: vaultValue]"),
        "{first_text}"
    );
    assert!(
        first_text.contains("home=none"),
        "the app's own key is not in the environment: {first_text}"
    );
    let shown = rig.confirmer.shown();
    let dialog = shown.last().unwrap();
    assert_eq!(dialog.method, "vibe-work/secretRun");
    assert_eq!(dialog.requester, Requester::Agent);
    assert_eq!(dialog.always.as_deref(), Some("Always"));
    let dialog_text = format!("{} {}", dialog.summary, dialog.params);
    assert!(
        dialog_text.contains("Deploy token") && dialog_text.contains("printer.sh"),
        "{dialog_text}"
    );
    assert!(!leaked(&dialog_text, VALUE), "{dialog_text}");
    // An admitted secret request must fit in full; the host adds its separate requester prefix.
    let native = vw_host::dialog_text(dialog).expect("the request and deadline fit one dialog");
    assert_eq!(
        native,
        format!("{}\n\n{}", vw_host::AGENT_LINE, dialog.summary)
    );
    assert!(!native.contains("printf"));
    assert_eq!(dialog.params["args"], json!(["printer.sh"]));
    assert_eq!(dialog.params["folder"], json!(project));
    assert!(
        dialog.summary.chars().count() <= vw_host::DIALOG_PARAMS_MAX_CHARS,
        "{native}"
    );
    // The folder is the project itself, so the project is named once (no separate Project line).
    assert_eq!(
        native
            .matches(&vw_gate::dialog_quote(&project.to_string_lossy()))
            .count(),
        1
    );
    assert!(!native.contains("Project:"), "{native}");
    for fact in [
        "Deploy token",
        "printer.sh",
        "Program:",
        "Arguments:",
        "Folder:",
        "Chat:",
    ] {
        assert!(native.contains(fact), "missing {fact}: {native}");
    }
    assert!(!native.contains("vibe-work/secretRun") && !native.contains("\"secrets\""));
    assert!(!leaked(&native, VALUE), "{native}");

    // Once means once: the next run asks again; Always sets the project permission.
    rig.confirmer.then(Confirmation::ApprovedAlways);
    let second = call(
        &who,
        "secret_run",
        run_args("sh", &["printer.sh"], env.clone()),
    )
    .await
    .unwrap();
    assert!(second.get("approval").is_none(), "{second}");
    assert_eq!(dialogs("vibe-work/secretRun"), 2);
    let third = call(
        &who,
        "secret_run",
        run_args("sh", &["printer.sh"], env.clone()),
    )
    .await
    .unwrap();
    assert!(third.get("approval").is_none(), "{third}");
    assert_eq!(
        dialogs("vibe-work/secretRun"),
        2,
        "an always grant asks nothing"
    );
    assert_eq!(
        service
            .entry_permission(project.to_str().unwrap(), &id)
            .await
            .unwrap()
            .0,
        vw_keys::EntryPermission::Allowed
    );
    // A grant for one secret never injects another one: the same command with a different secret asks.
    secrets(
        &rig.core,
        Origin::Ui,
        json!({"op": "add", "name": "Other token", "value": "vwOtherSecretValue9876543210ZYXwvu"}),
    )
    .await;
    rig.confirmer.then(Confirmation::Declined);
    let swapped_secret = call(
        &who,
        "secret_run",
        run_args(
            "sh",
            &["printer.sh"],
            json!([{"name": "DEPLOY_TOKEN", "secret": "Other token"}]),
        ),
    )
    .await;
    assert!(
        swapped_secret.unwrap_err().contains("the user said No"),
        "a dialog was asked"
    );
    assert_eq!(dialogs("vibe-work/secretRun"), 3, "another secret asks");
    let other = call(
        &who,
        "secret_run",
        run_args(
            env!("CARGO_BIN_EXE_secret_exec_helper"),
            &["printenv", "DEPLOY_TOKEN"],
            env.clone(),
        ),
    )
    .await
    .unwrap();
    assert!(other.get("approval").is_none(), "{other}");
    assert_eq!(
        dialogs("vibe-work/secretRun"),
        3,
        "a different command uses the same secret permission"
    );
    assert!(
        other["stdout"]
            .as_str()
            .unwrap()
            .contains("[secret withheld: vaultValue]"),
        "{other}"
    );
    assert!(!leaked(&other.to_string(), VALUE), "{other}");
    let echoed = call(
        &who,
        "secret_run",
        run_args(
            env!("CARGO_BIN_EXE_secret_exec_helper"),
            &["echo", "$DEPLOY_TOKEN"],
            env.clone(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        echoed["stdout"], "$DEPLOY_TOKEN\n",
        "no shell: nothing is expanded: {echoed}"
    );
    let refused = call(&who, "secret_run", run_args("env", &[], env.clone())).await;
    assert!(refused.is_err(), "a bare env is refused: {refused:?}");
    // Changing this project's choice immediately returns to asking.
    secrets(
        &rig.core,
        Origin::Ui,
        json!({"op":"setEntryPermission", "project":project, "id":id, "permission":"askEachTime"}),
    )
    .await;
    rig.confirmer.then(Confirmation::Declined);
    assert!(
        call(
            &who,
            "secret_run",
            run_args("sh", &["printer.sh"], env.clone())
        )
        .await
        .unwrap_err()
        .contains("the user said No")
    );
    assert_eq!(dialogs("vibe-work/secretRun"), 4);

    // Refused before any dialog.
    for (program, args, env, why) in [
        (
            "sh",
            vec!["-c", "echo $DEPLOY_TOKEN"],
            env.clone(),
            "inline",
        ),
        ("python3", vec!["-c", "print(1)"], env.clone(), "inline"),
        ("env", vec!["-S", "sh -c x"], env.clone(), "inline"),
        (
            "sh",
            vec!["printer.sh"],
            json!([{"name": "PATH", "secret": "Deploy token"}]),
            "changes how programs",
        ),
        (
            "sh",
            vec!["printer.sh"],
            json!([{"name": "X_KEY", "secret": "nope"}]),
            "no secret named",
        ),
        (
            "sh",
            vec!["printer.sh"],
            json!([{"name": "X_KEY", "secret": "Mistral API key (Vibe Work)"}]),
            "no secret named",
        ),
    ] {
        let refused = call(&who, "secret_run", run_args(program, &args, env)).await;
        let reason = refused.expect_err("refused");
        assert!(reason.contains(why), "{program} {args:?}: {reason}");
    }
    let mut outside = run_args("sh", &["printer.sh"], env.clone());
    outside["cwd"] = json!("/");
    let reason = call(&who, "secret_run", outside)
        .await
        .expect_err("outside");
    assert!(reason.contains("outside"), "{reason}");
    assert_eq!(
        dialogs("vibe-work/secretRun"),
        4,
        "no dialog for a refused request"
    );

    // A time limit stops the program.
    rig.confirmer.then(Confirmation::Approved);
    let mut slow = run_args("sh", &["sleeper.sh"], env.clone());
    slow["timeoutSeconds"] = json!(1);
    let started = std::time::Instant::now();
    let timed = call(&who, "secret_run", slow).await.unwrap();
    assert_eq!(timed["timedOut"], true, "{timed}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );

    // The original hook call stays pending while the native vault dialog is open.
    let before = rig.confirmer.shown().len();
    let hold = rig.confirmer.hold();
    rig.confirmer.then(Confirmation::Approved);
    let caller = who.clone();
    let args = run_args("sh", &["printer.sh"], env.clone());
    let pending = tokio::spawn(async move { agent_tools::call(&caller, "secret_run", args).await });
    rig.confirmer.wait_shown(before).await;
    assert!(!pending.is_finished());
    hold.notify_one();
    let done = pending.await.unwrap().unwrap();
    assert_eq!(done["exitCode"], 0, "{done}");
    assert!(!leaked(&done.to_string(), VALUE), "{done}");

    // The use log has every decision, no value.
    let listed = secrets(&rig.core, Origin::Ui, json!({"op": "list"})).await;
    let decisions: Vec<&str> = listed["uses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["decision"].as_str().unwrap())
        .collect();
    for d in ["allowedOnce", "allowedAlways", "entryAllowed", "declined"] {
        assert!(decisions.contains(&d), "{d} in {decisions:?}");
    }
    assert!(!leaked(&listed.to_string(), VALUE), "{listed}");

    // The session as the Debug API reads it (the tool answers sit in the transcript in the real app).
    let read: Command =
        serde_json::from_value(json!({"type": "readSession", "sessionId": sid})).unwrap();
    let session =
        serde_json::to_string(&rig.core.execute(Origin::DebugApi, read).await.unwrap()).unwrap();
    assert!(!leaked(&session, VALUE), "session read");

    // The user access switch blocks every secret tool while the cryptographic vault stays open.
    secrets(
        &rig.core,
        Origin::Ui,
        json!({"op":"setAgentAccess","locked":true}),
    )
    .await;
    let expected = "Not run: agent access to secrets is paused. There is no access to secrets until the user resumes it.";
    for (tool, args) in [
        ("secret_list", json!({})),
        ("secret_run", run_args("sh", &["./safe.sh"], env.clone())),
        ("secret_run_status", json!({"requestId":"missing"})),
    ] {
        assert_eq!(call(&who, tool, args).await.unwrap_err(), expected);
    }
    secrets(
        &rig.core,
        Origin::Ui,
        json!({"op":"setAgentAccess","locked":false}),
    )
    .await;
    assert!(call(&who, "secret_list", json!({})).await.is_ok());

    // A cryptographically locked vault asks for its password independently.
    service.lock();
    let locked = call(&who, "secret_list", json!({}))
        .await
        .expect_err("locked");
    assert!(locked.contains("locked"), "{locked}");

    // No log line carries the value or the Mistral key.
    let logged = String::from_utf8_lossy(&log.lock().unwrap()).into_owned();
    assert!(logged.contains("agent tool call"), "the capture works");
    assert!(!leaked(&logged, VALUE), "a log line carries the value");
    assert!(
        !leaked(&logged, MISTRAL_KEY),
        "a log line carries the Mistral key"
    );
}
