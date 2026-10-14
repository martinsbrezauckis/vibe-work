//! Secret broker behavior over a fake Vibe session and disposable vault.
//! PREDICT: user consent gates secret use, locked vaults refuse it, and answers withhold values.
#![cfg(unix)]

mod agent_tools;
mod common;
mod procmon;

use std::sync::{Arc, Mutex, OnceLock};

use agent_tools::{Dirs, call, credential_of, rig_with};
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
        .create_vault(SecretString::new(pass.as_str()), false)
        .await
        .unwrap();
    service
        .store_key(SecretString::new(MISTRAL_KEY))
        .await
        .unwrap();
    let svc = service.clone();
    let rig = rig_with(&dirs, "broker", move |c| c.keys = Some(svc)).await;
    let project = dirs.root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    // The program prints the value plainly, as base64 and inside JSON, and writes it to a side file (the falsifier:
    // the output really carried it).
    std::fs::write(
        project.join("printer.sh"),
        "printf 'plain=%s\\n' \"$DEPLOY_TOKEN\"\nprintf '%s' \"$DEPLOY_TOKEN\" | base64\n\
         printf '{\"k\":\"%s\"}\\n' \"$DEPLOY_TOKEN\"\nprintf '%s' \"$DEPLOY_TOKEN\" > proof.txt\n\
         printf 'home=%s\\n' \"${MISTRAL_API_KEY:-none}\"\n\
         printf '%s' \"$DEPLOY_TOKEN\" | od -An -tx1 | tr -d ' \\n'; echo\n",
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
    assert_eq!(first["approval"], "allowedOnce", "{first_text}");
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
    assert_eq!(dialog.always.as_deref(), Some("Always for this command"));
    let dialog_text = format!("{} {}", dialog.summary, dialog.params);
    assert!(
        dialog_text.contains("DEPLOY_TOKEN") && dialog_text.contains("printer.sh"),
        "{dialog_text}"
    );
    assert!(!leaked(&dialog_text, VALUE), "{dialog_text}");
    // Longer checkout paths can require the product's full-text pages instead of one dialog.
    let native = vw_host::dialog_text(dialog).unwrap_or_else(|| {
        let pages = vw_host::full_pages(dialog).expect("long agent requests have full-text pages");
        (1..=pages)
            .map(|page| {
                vw_host::shortened_dialog_text(dialog, vw_host::ShortStep::Page(page))
                    .expect("every full-text page exists")
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    for fact in [
        "DEPLOY_TOKEN",
        "printer.sh",
        "Program:",
        "Arguments (exact):",
        "Folder:",
    ] {
        assert!(native.contains(fact), "missing {fact}: {native}");
    }
    assert!(!native.contains("vibe-work/secretRun") && !native.contains("\"secrets\""));
    assert!(!leaked(&native, VALUE), "{native}");

    // Once means once: the next run asks again; "always" stores a grant.
    rig.confirmer.then(Confirmation::ApprovedAlways);
    let second = call(
        &who,
        "secret_run",
        run_args("sh", &["printer.sh"], env.clone()),
    )
    .await
    .unwrap();
    assert_eq!(second["approval"], "allowedAlways", "{second}");
    assert_eq!(dialogs("vibe-work/secretRun"), 2);
    let third = call(
        &who,
        "secret_run",
        run_args("sh", &["printer.sh"], env.clone()),
    )
    .await
    .unwrap();
    assert_eq!(third["approval"], "grant", "{third}");
    assert_eq!(
        dialogs("vibe-work/secretRun"),
        2,
        "an always grant asks nothing"
    );
    let listed = secrets(&rig.core, Origin::DebugApi, json!({"op": "list"})).await;
    let grant = &listed["grants"][0];
    assert_eq!(
        grant["bindings"][0], "DEPLOY_TOKEN <- Deploy token",
        "{listed}"
    );
    assert!(
        grant["command"].as_str().unwrap().ends_with("/printer.sh"),
        "{listed}"
    );
    assert_eq!(
        grant["uses"], 2,
        "the first run under the grant and this one: {listed}"
    );
    assert_eq!(
        grant["project"],
        project.to_string_lossy().as_ref(),
        "{listed}"
    );

    // The grant binds to the exact command: another command with the same secret
    // asks (falsifier: the first version's project-wide grant answered "grant" here); `printenv` and `echo` print the
    // value or not, the answer never holds it.
    let original = std::fs::read_to_string(project.join("printer.sh")).unwrap();
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
        swapped_secret.unwrap_err().contains("did not allow"),
        "a dialog was asked"
    );
    assert_eq!(dialogs("vibe-work/secretRun"), 3, "another secret asks");
    rig.confirmer.then(Confirmation::Approved);
    let other = call(
        &who,
        "secret_run",
        run_args("printenv", &["DEPLOY_TOKEN"], env.clone()),
    )
    .await
    .unwrap();
    assert_eq!(other["approval"], "allowedOnce", "{other}");
    assert_eq!(
        dialogs("vibe-work/secretRun"),
        4,
        "a different command asks again"
    );
    assert!(
        other["stdout"]
            .as_str()
            .unwrap()
            .contains("[secret withheld: vaultValue]"),
        "{other}"
    );
    assert!(!leaked(&other.to_string(), VALUE), "{other}");
    rig.confirmer.then(Confirmation::Approved);
    let echoed = call(
        &who,
        "secret_run",
        run_args("echo", &["$DEPLOY_TOKEN"], env.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        echoed["stdout"], "$DEPLOY_TOKEN\n",
        "no shell: nothing is expanded: {echoed}"
    );
    let refused = call(&who, "secret_run", run_args("env", &[], env.clone())).await;
    assert!(refused.is_err(), "a bare env is refused: {refused:?}");
    // A changed script is a different command: the grant does not cover it.
    std::fs::write(
        project.join("printer.sh"),
        format!(
            "{}\n",
            std::fs::read_to_string(project.join("printer.sh")).unwrap()
        ),
    )
    .unwrap();
    rig.confirmer.then(Confirmation::Approved);
    let changed = call(
        &who,
        "secret_run",
        run_args("sh", &["printer.sh"], env.clone()),
    )
    .await
    .unwrap();
    assert_eq!(changed["approval"], "allowedOnce", "{changed}");
    assert_eq!(dialogs("vibe-work/secretRun"), 6);
    // Time of check vs time of use: the script is rewritten while the dialog is open -> not run.
    let hold = rig.confirmer.hold();
    rig.confirmer.then(Confirmation::Approved);
    let who2 = who.clone();
    let pending = tokio::spawn(async move {
        call(
            &who2,
            "secret_run",
            run_args(
                "sh",
                &["printer.sh"],
                json!([{"name": "DEPLOY_TOKEN", "secret": "Deploy token"}]),
            ),
        )
        .await
    });
    while dialogs("vibe-work/secretRun") < 7 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    std::fs::remove_file(project.join("proof.txt")).unwrap();
    std::fs::write(
        project.join("printer.sh"),
        "printf '%s' \"$DEPLOY_TOKEN\" > proof.txt\n",
    )
    .unwrap();
    hold.notify_one();
    let swapped = pending.await.unwrap();
    assert!(
        swapped
            .as_ref()
            .unwrap_err()
            .contains("changed after the user allowed"),
        "{swapped:?}"
    );
    assert!(
        !project.join("proof.txt").exists(),
        "the rewritten script never ran"
    );
    // The script as it was when "always" was chosen: the grant covers it again.
    std::fs::write(project.join("printer.sh"), &original).unwrap();
    let again = call(
        &who,
        "secret_run",
        run_args("sh", &["printer.sh"], env.clone()),
    )
    .await
    .unwrap();
    assert_eq!(again["approval"], "grant", "{again}");
    assert_eq!(dialogs("vibe-work/secretRun"), 7);

    // Revoke: asked again; declined is refused.
    secrets(
        &rig.core,
        Origin::Ui,
        json!({"op": "revoke", "grantId": grant["id"]}),
    )
    .await;
    // Grants are whole ms; a revoke in the same ms as the next dialog's start voids its answer.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    rig.confirmer.then(Confirmation::Declined);
    let declined = call(
        &who,
        "secret_run",
        run_args("sh", &["printer.sh"], env.clone()),
    )
    .await;
    assert!(
        declined.as_ref().unwrap_err().contains("did not allow"),
        "{declined:?}"
    );
    assert_eq!(dialogs("vibe-work/secretRun"), 8);

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
        8,
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

    // A dialog left open: a request id, then the result.
    let hold = rig.confirmer.hold();
    rig.confirmer.then(Confirmation::Approved);
    // The binary waits 50 s for the core (wire::CALL_TIMEOUT); the core answers after 40 s.
    let (endpoint, credential) = who.clone();
    let args = run_args("sh", &["printer.sh"], env.clone());
    let waiting = tokio::task::spawn_blocking(move || {
        vw_agent_tools::wire::call(
            &endpoint,
            &credential,
            "secret_run",
            args,
            vw_agent_tools::wire::CALL_TIMEOUT,
        )
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        waiting["status"], "waiting for the user's answer",
        "{waiting}"
    );
    let request_id = waiting["requestId"].as_str().unwrap().to_owned();
    hold.notify_one();
    let done = call(&who, "secret_run_status", json!({"requestId": request_id}))
        .await
        .unwrap();
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
    for d in ["allowedOnce", "allowedAlways", "grant", "declined"] {
        assert!(decisions.contains(&d), "{d} in {decisions:?}");
    }
    assert!(!leaked(&listed.to_string(), VALUE), "{listed}");

    // The session as the Debug API reads it (the tool answers sit in the transcript in the real app).
    let read: Command =
        serde_json::from_value(json!({"type": "readSession", "sessionId": sid})).unwrap();
    let session =
        serde_json::to_string(&rig.core.execute(Origin::DebugApi, read).await.unwrap()).unwrap();
    assert!(!leaked(&session, VALUE), "session read");

    // Review of secret_pin.rs (follow-up): a link the command names, re-pointed after the consent. `sh ./run.sh` with
    // run.sh -> good.sh is allowed while its dialog is open; the link is then pointed at evil.sh. Falsifier: the first
    // version verified the pinned real path (good.sh, unchanged) and ran `sh ./run.sh`, i.e. evil.sh, with the secret.
    // Fixed: the run passes the pinned canonical path and verify refuses a link that now points elsewhere.
    std::fs::write(
        project.join("good.sh"),
        "printf '%s' \"$DEPLOY_TOKEN\" > good.txt\n",
    )
    .unwrap();
    std::fs::write(
        project.join("evil.sh"),
        "printf '%s' \"$DEPLOY_TOKEN\" > evil.txt\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("good.sh", project.join("run.sh")).unwrap();
    let hold = rig.confirmer.hold();
    rig.confirmer.then(Confirmation::Approved);
    let shown_before = dialogs("vibe-work/secretRun");
    let who3 = who.clone();
    let link_env = env.clone();
    let pending = tokio::spawn(async move {
        call(&who3, "secret_run", run_args("sh", &["./run.sh"], link_env)).await
    });
    while dialogs("vibe-work/secretRun") == shown_before {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let shown = rig.confirmer.shown();
    let link_dialog = shown.last().unwrap().params.to_string();
    assert!(
        link_dialog.contains("good.sh"),
        "the dialog shows the file that runs: {link_dialog}"
    );
    std::fs::remove_file(project.join("run.sh")).unwrap();
    std::os::unix::fs::symlink("evil.sh", project.join("run.sh")).unwrap();
    hold.notify_one();
    let repointed = pending.await.unwrap();
    assert!(
        !project.join("evil.txt").exists(),
        "evil.sh never ran with the secret: {repointed:?}"
    );
    assert!(
        repointed
            .as_ref()
            .err()
            .is_some_and(|e| e.contains("now points to another file"))
            || project.join("good.txt").exists(),
        "refused, or the approved good.sh ran: {repointed:?}"
    );
    // With an "always" grant for `sh ./run.sh` -> evil.sh as it stands now, a later re-point to good.sh is a different
    // command: it asks again.
    std::fs::remove_file(project.join("run.sh")).unwrap();
    std::os::unix::fs::symlink("good.sh", project.join("run.sh")).unwrap();
    rig.confirmer.then(Confirmation::ApprovedAlways);
    let granted = call(
        &who,
        "secret_run",
        run_args("sh", &["./run.sh"], env.clone()),
    )
    .await
    .unwrap();
    assert_eq!(granted["approval"], "allowedAlways", "{granted}");
    std::fs::remove_file(project.join("run.sh")).unwrap();
    std::os::unix::fs::symlink("evil.sh", project.join("run.sh")).unwrap();
    let before = dialogs("vibe-work/secretRun");
    rig.confirmer.then(Confirmation::Declined);
    let after_repoint = call(
        &who,
        "secret_run",
        run_args("sh", &["./run.sh"], env.clone()),
    )
    .await;
    assert!(after_repoint.is_err(), "{after_repoint:?}");
    assert_eq!(
        dialogs("vibe-work/secretRun"),
        before + 1,
        "a re-pointed link asks again"
    );
    assert!(!project.join("evil.txt").exists(), "evil.sh never ran");

    // Review of the link fix (validator differential): every check, the dialog and the run read ONE final argv (the
    // pinned canonical program and paths). (a) A link `./tool` -> `bin/shutdown`: the safety reminder sees `shutdown`
    // and refuses before any dialog (falsifier: checks on the argv as written saw `./tool`, asked, and ran the
    // script). (b) `sh ./safe.sh` with safe.sh -> danger.sh: the dialog shows danger.sh, the file that runs.
    std::fs::create_dir_all(project.join("bin")).unwrap();
    std::fs::write(
        project.join("bin/shutdown"),
        "#!/bin/sh\necho ran > shut.txt\n",
    )
    .unwrap();
    std::fs::set_permissions(
        project.join("bin/shutdown"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    std::os::unix::fs::symlink("bin/shutdown", project.join("tool")).unwrap();
    let before = dialogs("vibe-work/secretRun");
    let stopped = call(&who, "secret_run", run_args("./tool", &[], env.clone())).await;
    assert!(
        stopped
            .as_ref()
            .err()
            .is_some_and(|e| e.contains("safety reminder")),
        "the reminder reads what runs: {stopped:?}"
    );
    assert_eq!(
        dialogs("vibe-work/secretRun"),
        before,
        "refused before any dialog"
    );
    assert!(!project.join("shut.txt").exists(), "the script never ran");
    std::fs::write(
        project.join("danger.sh"),
        "# would delete the home folder here\necho danger\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("danger.sh", project.join("safe.sh")).unwrap();
    rig.confirmer.then(Confirmation::Declined);
    let _ = call(
        &who,
        "secret_run",
        run_args("sh", &["./safe.sh"], env.clone()),
    )
    .await;
    let shown = rig.confirmer.shown();
    let runs = shown.last().unwrap().params["runs exactly (no shell)"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        runs.ends_with("/danger.sh") && !runs.contains("safe.sh"),
        "the dialog shows what runs: {runs}"
    );

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
