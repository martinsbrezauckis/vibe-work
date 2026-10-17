//! Secret command grants over a fake Vibe session and a disposable vault.
//! PREDICT: forged grants and revoked grants still ask; revocation during consent wins.
#![cfg(unix)]

mod agent_tools;
mod common;
mod procmon;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_tools::{Dirs, Rig, call, credential_of, rig_with};
use common::vault::{client, key_service_config, memory_store};
use serde_json::{Value, json};
use vw_core_api::{Command, CommandResult, Origin};
use vw_gate::Confirmation;
use vw_keys::KeyService;
use vw_keys::existing::MemoryVibeKeyring;
use vw_vault::SecretString;
use vw_vault::command_grant::{id_for, resource};
use vw_vault::items::ResourcePermission;

const VALUE: &str = "vwSg1FakeDeployToken0123456789ABCDEFqrs";
const METHOD: &str = "vibe-work/secretRun";

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap()
}

async fn secrets(rig: &Rig, request: Value) -> Value {
    let command: Command =
        serde_json::from_value(json!({"type": "secrets", "request": request})).unwrap();
    match rig.core.execute(Origin::Ui, command).await.unwrap() {
        CommandResult::Secrets { result } => serde_json::to_value(result).unwrap(),
        other => panic!("{other:?}"),
    }
}

fn run_args(program: &str, args: &[&str]) -> Value {
    json!({
        "program": program,
        "args": args,
        "env": [{"name": "DEPLOY_TOKEN", "secret": "Deploy token"}],
        "purpose": "deploy the test site"
    })
}

fn dialogs(rig: &Rig) -> usize {
    rig.confirmer
        .shown()
        .iter()
        .filter(|r| r.method == METHOD)
        .count()
}

async fn key_handle(kdir: &Path, pass: &str, create: bool) -> Arc<KeyService> {
    let service = KeyService::start(key_service_config(
        kdir,
        "http://127.0.0.1:9",
        Arc::new(MemoryVibeKeyring::default()),
    ))
    .await
    .unwrap();
    if create {
        service
            .create_vault(SecretString::new(pass), false)
            .await
            .unwrap();
    } else {
        service
            .unlock(SecretString::new(pass), None, None)
            .await
            .unwrap();
    }
    service
}

/// The revoke another window or app writes (as `secret_grants::revoke` does).
async fn revoke_elsewhere(other: &KeyService, id: &str) {
    let at = now();
    other
        .update_command_grant(id.to_owned(), move |current| {
            let mut g = current?;
            g.always = false;
            g.revoked_ms = g.revoked_ms.max(at).max(g.granted_ms);
            Some(g)
        })
        .await
        .unwrap()
        .expect("the record exists");
}

struct Setup {
    dirs: Dirs,
    kdir: PathBuf,
    service: Arc<KeyService>,
    project: PathBuf,
    pass: String,
}

async fn setup(name: &str) -> Setup {
    let dirs = Dirs::new(name);
    vw_changes::prepare_root(&dirs.root.join("changes")).unwrap();
    let kdir = dirs.root.join("keys");
    std::fs::create_dir_all(&kdir).unwrap();
    let pass = format!("test-passphrase-{}", vw_vault::login::new_id());
    let service = key_handle(&kdir, &pass, true).await;
    let project = dirs.root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    // The program writes the value to a side file: present = it ran with the secret.
    std::fs::write(
        project.join("deploy.sh"),
        "printf '%s' \"$DEPLOY_TOKEN\" > proof.txt\necho deployed\n",
    )
    .unwrap();
    Setup {
        dirs,
        kdir,
        service,
        project,
        pass,
    }
}

async fn core(s: &Setup, tag: &str) -> Rig {
    let svc = s.service.clone();
    rig_with(&s.dirs, tag, move |c| c.keys = Some(svc)).await
}

fn proof(s: &Setup) -> PathBuf {
    s.project.join("proof.txt")
}

fn ran(s: &Setup) -> bool {
    let p = proof(s);
    let ran = p.exists();
    if ran {
        assert_eq!(std::fs::read_to_string(&p).unwrap(), VALUE);
        std::fs::remove_file(&p).unwrap();
    }
    ran
}

fn leaked(text: &str) -> bool {
    vw_secrets::forms(VALUE)
        .iter()
        .any(|f| text.as_bytes().windows(f.len()).any(|w| w == f.as_slice()))
}

fn manifests(kdir: &Path) -> PathBuf {
    let dir = kdir.join("shellx-vault/local-vault-server/repos/default/manifests");
    assert!(dir.is_dir(), "{}", dir.display());
    dir
}

fn head_generation(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.unwrap().file_name().to_str()?.parse::<u64>().ok())
        .max()
        .unwrap()
}

/// The pre-grant file holding `grant` (a `secrets list` grant row) as an active grant.
fn plant_file(data: &Path, grant: &Value) {
    let file = json!({"version": 1, "grants": [grant], "log": [], "next": 7});
    std::fs::write(data.join("secret-grants.json"), file.to_string()).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn a_planted_forged_replayed_or_revoked_grant_never_skips_the_dialog() {
    let s = setup("forged-grant").await;
    let rig = core(&s, "grant-initial").await;
    rig.start_session(&s.project, "ask").await;
    let who = credential_of(&rig.injected(0));
    secrets(
        &rig,
        json!({"op": "add", "name": "Deploy token", "value": VALUE}),
    )
    .await;
    let mut answers: Vec<String> = Vec::new();

    // A real grant, its row as Settings lists it.
    rig.confirmer.then(Confirmation::ApprovedAlways);
    let first = call(&who, "secret_run", run_args("sh", &["deploy.sh"]))
        .await
        .unwrap();
    assert_eq!(first["approval"], "allowedAlways", "{first}");
    assert!(ran(&s));
    let again = call(&who, "secret_run", run_args("sh", &["deploy.sh"]))
        .await
        .unwrap();
    assert_eq!(again["approval"], "grant", "{again}");
    assert!(ran(&s));
    assert_eq!(dialogs(&rig), 1);
    let listed = secrets(&rig, json!({"op": "list"})).await;
    let row = listed["grants"][0].clone();
    let id = row["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("vibe-work-grant-"), "{listed}");
    let fingerprint = row["fingerprint"].as_str().unwrap().to_owned();
    let project = row["project"].as_str().unwrap().to_owned();
    assert_eq!(id, id_for(&project, &fingerprint));
    answers.push(listed.to_string());

    // Another window revokes: the next run asks.
    let other = key_handle(&s.kdir, &s.pass, false).await;
    revoke_elsewhere(&other, &id).await;
    common::revoke::after_revoke(&s.service, &id).await;
    rig.confirmer.then(Confirmation::Declined);
    let asked = call(&who, "secret_run", run_args("sh", &["deploy.sh"])).await;
    assert!(
        asked.as_ref().unwrap_err().contains("did not allow"),
        "{asked:?}"
    );
    assert_eq!(
        dialogs(&rig),
        2,
        "a revoke elsewhere is seen by the next run"
    );
    assert!(!ran(&s));

    // The old file, planted while the core runs: nothing reads it.
    plant_file(&s.dirs.data, &row);
    rig.confirmer.then(Confirmation::Declined);
    let planted = call(&who, "secret_run", run_args("sh", &["deploy.sh"])).await;
    assert!(planted.is_err(), "{planted:?}");
    assert_eq!(dialogs(&rig), 3, "a planted file grants nothing");
    assert!(!ran(&s), "the program never ran");

    // A grant-shaped record that is not userOnly, at the derived id, written by another vault app.
    let raw = client(&s.kdir.join("shellx-vault"), memory_store());
    raw.unlock(&SecretString::new(s.pass.as_str()), None, None)
        .unwrap();
    let mut forged = other.command_grant(id.clone()).await.unwrap().unwrap();
    forged.always = true;
    forged.granted_ms = now() + 1;
    let mut forged_resource = resource(&forged, now());
    forged_resource.permission = ResourcePermission::VisibleAsk;
    raw.upsert_item(&forged_resource.to_item().unwrap())
        .unwrap();
    rig.confirmer.then(Confirmation::Declined);
    let forged_run = call(&who, "secret_run", run_args("sh", &["deploy.sh"])).await;
    assert!(forged_run.is_err(), "{forged_run:?}");
    assert_eq!(dialogs(&rig), 4, "a visibleAsk record is no grant");
    assert!(!ran(&s));
    // "Always" replaces it with a real grant.
    rig.confirmer.then(Confirmation::ApprovedAlways);
    let regranted = call(&who, "secret_run", run_args("sh", &["deploy.sh"]))
        .await
        .unwrap();
    assert_eq!(regranted["approval"], "allowedAlways", "{regranted}");
    assert!(ran(&s));
    let granted = call(&who, "secret_run", run_args("sh", &["deploy.sh"]))
        .await
        .unwrap();
    assert_eq!(granted["approval"], "grant", "{granted}");
    assert!(ran(&s));
    assert_eq!(dialogs(&rig), 5);

    // Replay: the version with the active grant, copied under the next number after a revoke.
    let m = manifests(&s.kdir);
    let n = head_generation(&m);
    let active_version = std::fs::read(m.join(format!("{n:08}"))).unwrap();
    secrets(&rig, json!({"op": "revoke", "grantId": id})).await;
    common::revoke::after_revoke(&s.service, &id).await;
    let revoked_at = head_generation(&m);
    assert!(revoked_at > n);
    let replay = m.join(format!("{:08}", revoked_at + 1));
    std::fs::write(&replay, &active_version).unwrap();
    let replayed = call(&who, "secret_run", run_args("sh", &["deploy.sh"])).await;
    eprintln!("replayed: {replayed:?}");
    assert!(replayed.is_err(), "refused: {replayed:?}");
    assert_eq!(dialogs(&rig), 5, "refused before any dialog");
    assert!(!ran(&s), "the program never ran");
    std::fs::remove_file(&replay).unwrap();
    rig.confirmer.then(Confirmation::Declined);
    let after_replay = call(&who, "secret_run", run_args("sh", &["deploy.sh"])).await;
    assert!(after_replay.is_err(), "{after_replay:?}");
    assert_eq!(dialogs(&rig), 6, "the real head: revoked, so it asks");
    assert!(!ran(&s));

    // A new core on the same data folder: the planted file is removed at start, and the run asks.
    let listed = secrets(&rig, json!({"op": "list"})).await;
    answers.push(listed.to_string());
    assert_eq!(
        listed["grants"].as_array().unwrap().len(),
        0,
        "a revoked grant is not listed: {listed}"
    );
    rig.core.shutdown().await;
    plant_file(&s.dirs.data, &row);
    let rig2 = core(&s, "grant-restarted").await;
    assert!(
        !s.dirs.data.join("secret-grants.json").exists(),
        "the old file is removed at start"
    );
    rig2.start_session(&s.project, "ask").await;
    let who2 = credential_of(&rig2.injected(0));
    rig2.confirmer.then(Confirmation::Declined);
    let restarted = call(&who2, "secret_run", run_args("sh", &["deploy.sh"])).await;
    assert!(restarted.is_err(), "{restarted:?}");
    assert_eq!(dialogs(&rig2), 1, "after a restart the dialog asks");
    assert!(!ran(&s), "the program never ran");

    for (i, text) in answers.iter().enumerate() {
        assert!(!leaked(text), "answer {i}");
    }
    for request in rig
        .confirmer
        .shown()
        .iter()
        .chain(rig2.confirmer.shown().iter())
    {
        assert!(!leaked(&format!("{} {}", request.summary, request.params)));
    }
    rig2.core.shutdown().await;
    other.shutdown().await;
}

/// Local commits use Vibe's access-mode card, with no git-guard dialog.
const COMMIT: [&str; 5] = ["commit", "--allow-empty", "-q", "-m", "fixture"];
/// A branch rewrite outside a session copy needs the git guard's approval in every mode.
const HELD_REWRITE: [&str; 4] = ["branch", "-f", "fixture-held", "HEAD"];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn a_locked_vault_at_the_grant_lookup_asks() {
    let s = setup("locked-grant").await;
    // A repository of its own: no copy branch, but ordinary commits still have no git-guard dialog.
    for args in [
        &["init", "-q"][..],
        &["config", "user.email", "t@example.invalid"][..],
        &["config", "user.name", "T"][..],
    ] {
        let done = std::process::Command::new("git")
            .args(args)
            .current_dir(&s.project)
            .status()
            .unwrap();
        assert!(done.success());
    }
    let rig = core(&s, "grant-locked").await;
    rig.start_session(&s.project, "auto-approve").await;
    let who = credential_of(&rig.injected(0));
    secrets(
        &rig,
        json!({"op": "add", "name": "Deploy token", "value": VALUE}),
    )
    .await;
    let git_dialogs = || {
        rig.confirmer
            .shown()
            .iter()
            .filter(|r| r.method == "vibe-work/gitGuard")
            .count()
    };

    // Only the secret dialog asks: "always" must be its answer, with no unused guard answer ahead of it.
    rig.confirmer.then(Confirmation::ApprovedAlways);
    let first = call(&who, "secret_run", run_args("git", &COMMIT))
        .await
        .unwrap();
    assert_eq!(first["approval"], "allowedAlways", "{first}");
    assert_eq!((git_dialogs(), dialogs(&rig)), (0, 1));
    let granted = call(&who, "secret_run", run_args("git", &COMMIT))
        .await
        .unwrap();
    assert_eq!(granted["approval"], "grant", "{granted}");
    assert_eq!((git_dialogs(), dialogs(&rig)), (0, 1));

    // The retained branch-rewrite guard provides the pause before grant lookup. Its exact command needs its own
    // grant: the plain commit's grant must not cover the rewrite.
    rig.confirmer.then(Confirmation::Approved);
    rig.confirmer.then(Confirmation::ApprovedAlways);
    let held_first = call(&who, "secret_run", run_args("git", &HELD_REWRITE))
        .await
        .unwrap();
    assert_eq!(held_first["approval"], "allowedAlways", "{held_first}");
    assert_eq!((git_dialogs(), dialogs(&rig)), (1, 2));

    // locked while the git guard's dialog is open means the entry's Never policy cannot be checked.
    // Refuse before secret consent, even though the older grant path would have asked.
    let hold = rig.confirmer.hold();
    rig.confirmer.then(Confirmation::Approved);
    let who2 = who.clone();
    let pending =
        tokio::spawn(
            async move { call(&who2, "secret_run", run_args("git", &HELD_REWRITE)).await },
        );
    tokio::time::timeout(Duration::from_secs(30), async {
        while git_dialogs() < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the retained branch-rewrite guard asks before grant lookup");
    s.service.lock();
    hold.notify_one();
    let locked = pending.await.unwrap();
    eprintln!("locked: {locked:?}");
    assert!(
        locked
            .as_ref()
            .unwrap_err()
            .contains("project permission could not be checked"),
        "{locked:?}"
    );
    assert_eq!(
        (git_dialogs(), dialogs(&rig)),
        (2, 2),
        "a locked vault refuses entry use"
    );

    // Unlocked again, the grant holds.
    s.service
        .unlock(SecretString::new(s.pass.as_str()), None, None)
        .await
        .unwrap();
    rig.confirmer.then(Confirmation::Approved);
    let back = call(&who, "secret_run", run_args("git", &HELD_REWRITE))
        .await
        .unwrap();
    assert_eq!(back["approval"], "grant", "{back}");
    assert_eq!((git_dialogs(), dialogs(&rig)), (3, 2));
    let plain_back = call(&who, "secret_run", run_args("git", &COMMIT))
        .await
        .unwrap();
    assert_eq!(plain_back["approval"], "grant", "{plain_back}");
    assert_eq!((git_dialogs(), dialogs(&rig)), (3, 2));
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoke_while_the_dialog_is_open_wins() {
    let s = setup("grant-revoke-race").await;
    let rig = core(&s, "grant-race").await;
    rig.start_session(&s.project, "ask").await;
    let who = credential_of(&rig.injected(0));
    secrets(
        &rig,
        json!({"op": "add", "name": "Deploy token", "value": VALUE}),
    )
    .await;
    let other = key_handle(&s.kdir, &s.pass, false).await;
    rig.confirmer.then(Confirmation::ApprovedAlways);
    call(&who, "secret_run", run_args("sh", &["deploy.sh"]))
        .await
        .unwrap();
    assert!(ran(&s));
    let listed = secrets(&rig, json!({"op": "list"})).await;
    let id = listed["grants"][0]["id"].as_str().unwrap().to_owned();
    secrets(&rig, json!({"op": "revoke", "grantId": id})).await;
    common::revoke::after_revoke(&s.service, &id).await;

    for answer in [Confirmation::ApprovedAlways, Confirmation::Approved] {
        let before = dialogs(&rig);
        let hold = rig.confirmer.hold();
        rig.confirmer.then(answer);
        let who2 = who.clone();
        let pending =
            tokio::spawn(
                async move { call(&who2, "secret_run", run_args("sh", &["deploy.sh"])).await },
            );
        common::revoke::dialog_after(|| dialogs(&rig), before).await;
        revoke_elsewhere(&other, &id).await;
        common::revoke::after_revoke(&s.service, &id).await;
        hold.notify_one();
        let out = pending.await.unwrap();
        eprintln!("{answer:?}: {out:?}");
        assert!(
            out.as_ref()
                .is_err_and(|e| e.contains("while the dialog was open")),
            "{answer:?}: the revoke wins: {out:?}"
        );
        let record = other.command_grant(id.clone()).await.unwrap().unwrap();
        assert!(!record.active(), "{record:?}");
        assert!(!ran(&s), "{answer:?}: the program never ran");
    }

    let uses = secrets(&rig, json!({"op": "list"})).await;
    let refused = uses["uses"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|u| u["decision"] == "refused")
        .count();
    assert_eq!(refused, 2, "the log says refused: {uses}");

    // Without a revoke in between, "always" re-activates the record.
    common::revoke::after_revoke(&s.service, &id).await;
    rig.confirmer.then(Confirmation::ApprovedAlways);
    let regranted = call(&who, "secret_run", run_args("sh", &["deploy.sh"]))
        .await
        .unwrap();
    assert_eq!(regranted["approval"], "allowedAlways", "{regranted}");
    assert!(ran(&s));
    let granted = call(&who, "secret_run", run_args("sh", &["deploy.sh"]))
        .await
        .unwrap();
    assert_eq!(granted["approval"], "grant", "{granted}");
    assert!(ran(&s));
    rig.core.shutdown().await;
    other.shutdown().await;
}

#[tokio::test]
async fn next_dialog_waits_until_wall_clock_passes_revoke() {
    // A revoke can be ahead of the local clock after an adjustment or a monotonic grant timestamp.
    let revoked = common::revoke::now_ms() + 1000;
    common::revoke::clock_after(revoked).await;
    assert!(common::revoke::now_ms() > revoked);
}
