//! Waiting, record and policy contracts with a fake native presenter and Tokio's injected time.
use super::*;
use crate::{confirmer::DynConfirmer, names::Names};
use serde_json::json;
use std::sync::Arc;
use vw_ask::{AnsweredBy, Choice, Outcome, Scope};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer, Requester};

fn service() -> Arc<Service> {
    Service::new(
        Arc::new(SystemAskClock::default()),
        tokio::sync::broadcast::channel(4096).0,
    )
}
#[tokio::test(start_paused = true)]
async fn unanswered_cards_wait_past_ten_minutes() {
    let s = service();
    let mut card = s.open(
        "approval",
        None,
        AnsweredBy::Person,
        vec![Choice::Once],
        None,
    );
    tokio::time::advance(std::time::Duration::from_secs(7_200)).await;
    assert_eq!(s.list().len(), 1);
    assert_eq!(s.records(), [] as [vw_ask::AskRecord; 0]);
    assert!(s.settle(&card.ask.id, allowed(AnsweredBy::Person, Scope::Once)));
    assert_eq!(
        card.closed().await,
        allowed(AnsweredBy::Person, Scope::Once)
    );
}
#[derive(Clone)]
struct Held(Arc<tokio::sync::Notify>);
impl Confirmer for Held {
    async fn confirm(&self, _: &ConfirmationRequest) -> Confirmation {
        self.0.notified().await;
        Confirmation::Approved
    }
}
fn request(method: &str, requester: Requester) -> ConfirmationRequest {
    ConfirmationRequest {
        method: method.into(),
        requester,
        summary: "Allow this request?".into(),
        params: json!({"sessionId":"session", "value":"never put this in a record"}),
        always: None,
    }
}
#[tokio::test(start_paused = true)]
async fn native_dialog_waits_and_accepts_an_answer_after_two_hours() {
    let s = service();
    let answer = Arc::new(tokio::sync::Notify::new());
    let confirmer = DynConfirmer::new(
        Held(answer.clone()),
        Arc::new(Names::default()),
        vw_session::SessionStore::new(),
        s.clone(),
    );
    let pending = tokio::spawn(async move {
        confirmer
            .confirm(&request("vibe-work/gitGuard", Requester::Agent))
            .await
    });
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(7_200)).await;
    assert_eq!(s.list().len(), 1);
    assert!(!pending.is_finished());
    answer.notify_one();
    assert_eq!(pending.await.unwrap(), Confirmation::Approved);
    assert_eq!(s.list(), [] as [vw_ask::Ask; 0]);
    assert_eq!(s.records().len(), 1);
}
#[tokio::test(start_paused = true)]
async fn questions_wait_until_skipped() {
    let s = service();
    let mut card = s.open(
        "user_input",
        None,
        AnsweredBy::Person,
        vec![Choice::No],
        None,
    );
    tokio::time::advance(std::time::Duration::from_secs(7_200)).await;
    assert_eq!(s.list().len(), 1);
    let outcome = Outcome::Cancelled {
        reason: "the question was skipped".into(),
    };
    assert!(s.settle(&card.ask.id, outcome.clone()));
    assert_eq!(card.closed().await, outcome);
}
#[tokio::test]
async fn every_native_policy_and_both_card_kinds_leave_exactly_one_record() {
    let s = service();
    let confirmer = DynConfirmer::new(
        Decline,
        Arc::new(Names::default()),
        vw_session::SessionStore::new(),
        s.clone(),
    );
    let mut expected = 0;
    for kind in
        vw_ask::policies().filter(|k| vw_ask::policy(k).unwrap().surface == vw_ask::Surface::Native)
    {
        for requester in [Requester::Window, Requester::DebugApi, Requester::Core] {
            let answer = confirmer.confirm(&request(kind, requester)).await;
            expected += 1;
            let records = s.records();
            assert_eq!(records.len(), expected, "each native ask leaves one record");
            let record = records.last().unwrap().clone();
            let automatic = requester != Requester::Window && !record.ask.person_only;
            assert_eq!(answer.approved(), automatic, "{kind}");
            assert_eq!(
                record.outcome,
                if automatic {
                    allowed(
                        if requester == Requester::DebugApi {
                            AnsweredBy::DebugApi
                        } else {
                            AnsweredBy::Core
                        },
                        Scope::Once,
                    )
                } else {
                    refused(AnsweredBy::Person, None)
                }
            );
        }
    }
    for kind in ["approval", "user_input"] {
        let handle = s.open(
            kind,
            None,
            vw_ask::Requester::Agent,
            vec![Choice::Once, Choice::No],
            None,
        );
        handle.finish(allowed(AnsweredBy::Person, Scope::Once));
        assert!(!s.settle(&handle.ask.id, refused(AnsweredBy::Person, None)));
        expected += 1;
    }
    assert_eq!(s.records().len(), expected);
}
#[test]
fn native_policy_does_not_drift_from_gate_or_feature_methods() {
    for (method, class) in vw_gate::classified_methods() {
        if class == vw_gate::Class::Confirmed {
            assert!(vw_ask::policy(method).is_some(), "missing policy: {method}");
        }
    }
    for method in crate::vault_scope::DIALOG_METHODS {
        assert!(vw_ask::policy(method).unwrap().person_only);
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    scan(&root);
}

fn scan(folder: &std::path::Path) {
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(&path);
            continue;
        }
        if path.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(path).unwrap();
        for line in source.lines().filter(|l| {
            (l.trim_start().starts_with("const ") || l.trim_start().starts_with("pub"))
                && l.contains("METHOD: &str =")
                && l.contains("vibe-work/")
        }) {
            let method = line.split('"').nth(1).unwrap();
            assert!(vw_ask::policy(method).is_some(), "missing policy: {method}");
        }
    }
}

struct Decline;
impl Confirmer for Decline {
    fn confirm(
        &self,
        _: &ConfirmationRequest,
    ) -> impl std::future::Future<Output = Confirmation> + Send {
        std::future::ready(Confirmation::Declined)
    }
}
#[tokio::test]
async fn records_keep_only_the_newest_five_hundred_and_scrub_plain_notes() {
    let secret = "ask-note-synthetic-value-981f6a";
    vw_secrets::register(vw_secrets::SecretKind::VaultValue, secret);
    let scrubbed = refused(AnsweredBy::Person, Some(format!("plain *note* {secret}")));
    assert!(!serde_json::to_string(&scrubbed).unwrap().contains(secret));
    let s = service();
    for _ in 0..510 {
        let h = s.open("approval", None, AnsweredBy::Person, vec![Choice::No], None);
        h.finish(refused(
            AnsweredBy::Person,
            Some("  a plain *note*  ".into()),
        ));
    }
    let records = s.records();
    assert_eq!(records.len(), 500);
    assert_eq!(records[0].ask.id, "ask-11");
    assert_eq!(
        records[0].outcome,
        refused(AnsweredBy::Person, Some("a plain *note*".into()))
    );
}

/// Linux `CLOCK_MONOTONIC` and macOS mach time stop while the machine sleeps. A prompt shown as
/// The timestamp follows wall time and the prompt remains answerable.
#[tokio::test]
async fn a_yes_after_machine_sleep_is_accepted() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static WALL: AtomicU64 = AtomicU64::new(1_700_000_000_000);
    fn wall() -> u64 {
        WALL.load(Ordering::SeqCst)
    }
    let clock = Arc::new(SystemAskClock::with_wall(wall));
    let s = Service::new(clock.clone(), tokio::sync::broadcast::channel(10).0);
    let handle = s.open(
        "vibe-work/gitGuard",
        None,
        AnsweredBy::Person,
        vec![Choice::Once, Choice::No],
        None,
    );
    // Two hours of sleep: the wall clock moves, the monotonic timer does not.
    WALL.fetch_add(2 * 60 * 60 * 1000, Ordering::SeqCst);
    assert!(
        clock.now_ms() >= handle.ask.opened_at + 7_200_000,
        "the ask clock must follow the wall clock across a sleep"
    );
    assert!(
        s.settle(&handle.ask.id, allowed(AnsweredBy::Person, Scope::Once)),
        "a yes after two hours still counts"
    );
    assert_eq!(
        s.records()[0].outcome,
        allowed(AnsweredBy::Person, Scope::Once)
    );
    // A wall clock set back never extends a permission: the clock never runs backwards.
    let before = clock.now_ms();
    WALL.fetch_sub(5 * 60 * 60 * 1000, Ordering::SeqCst);
    assert!(clock.now_ms() >= before);
}

/// The confirmer takes a guard's ask id from `params.askId`. A second request naming the same id
/// must get its own ask and its own answer; it must never ride on another request's yes (here: a vault request
/// approved by a Git dialog's yes, without its own person-only dialog being answered).
#[tokio::test(start_paused = true)]
async fn a_request_naming_another_asks_id_never_shares_its_answer() {
    struct ByMethod(Arc<tokio::sync::Notify>);
    impl Confirmer for ByMethod {
        async fn confirm(&self, request: &ConfirmationRequest) -> Confirmation {
            if request.method == "vibe-work/gitGuard" {
                self.0.notified().await;
                Confirmation::Approved
            } else {
                std::future::pending().await
            }
        }
    }
    let s = service();
    let yes = Arc::new(tokio::sync::Notify::new());
    let confirmer = DynConfirmer::new(
        ByMethod(yes.clone()),
        Arc::new(Names::default()),
        vw_session::SessionStore::new(),
        s.clone(),
    );
    let mut git = request("vibe-work/gitGuard", Requester::Agent);
    git.params["askId"] = json!("guard-shared");
    let mut vault = request("vibe-work/secretRun", Requester::Agent);
    vault.params["askId"] = json!("guard-shared");
    let first = confirmer.clone();
    let git_answer = tokio::spawn(async move { first.confirm(&git).await });
    tokio::task::yield_now().await;
    let second = confirmer.clone();
    let vault_answer = tokio::spawn(async move { second.confirm(&vault).await });
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    assert_eq!(s.list().len(), 2, "each request has its own ask");
    yes.notify_one();
    assert_eq!(git_answer.await.unwrap(), Confirmation::Approved);
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    assert!(
        !vault_answer.is_finished(),
        "the vault request still waits for its own answer"
    );
    let open = s.list();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].kind, "vibe-work/secretRun");
    assert!(open[0].person_only);
    vault_answer.abort();
}

/// A secret that straddles the 2,000-character cut must not leave its first characters behind.
#[test]
fn a_secret_cut_by_the_note_limit_is_still_withheld() {
    let secret = "ask-cut-synthetic-value-4d1c9e7a52b3";
    vw_secrets::register(vw_secrets::SecretKind::VaultValue, secret);
    let note = format!("{}{secret}", "x".repeat(1990));
    let scrubbed = service::scrub_note(&note);
    assert!(!scrubbed.contains(&secret[..10]), "{}", &scrubbed[1980..]);
    assert!(scrubbed.chars().count() <= 2000);
}

/// There are no permission timeout settings.
#[test]
fn removed_deadline_commands_are_refused() {
    assert!(
        serde_json::from_value::<crate::Command>(
            json!({"type":"askSettings", "request":{"op":"write","minutes":60}})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<crate::Command>(
            json!({"type":"extendAsk","id":"callback:x","deadline":1})
        )
        .is_err()
    );
}

#[test]
fn built_in_session_grants_describe_the_whole_action_in_plain_words() {
    for (tool, action) in [
        (
            "file_system.write_file",
            "creating and changing files anywhere in this session",
        ),
        (
            "file_system.read_file",
            "reading files anywhere in this session",
        ),
        (
            "file_system.edit",
            "changing files anywhere in this session",
        ),
        (
            "file_system.grep",
            "searching file contents anywhere in this session",
        ),
        ("web_search", "searching the web in this session"),
        (
            "file_system.search_replace",
            "changing files anywhere in this session",
        ),
        ("skill.read", "reading skill instructions in this session"),
        ("subagent.spawn", "starting subagents in this session"),
        ("agents.spawn", "starting subagents in this session"),
        ("agents.list", "listing subagents in this session"),
        (
            "agents.list_configs",
            "listing agent configurations in this session",
        ),
        (
            "agents.send_message",
            "sending messages to subagents in this session",
        ),
        ("agents.interrupt", "interrupting subagents in this session"),
        ("self.wait", "waiting in this session"),
        ("subagent.list", "listing subagents in this session"),
        ("subagent.wait", "waiting for subagents in this session"),
        (
            "subagent.send_message",
            "sending messages to subagents in this session",
        ),
        (
            "subagent.interrupt",
            "interrupting subagents in this session",
        ),
        ("subagent.stop", "stopping subagents in this session"),
        ("self.sleep", "waiting in this session"),
    ] {
        let entry = json!({"detail":{"kind":"approval", "effect":{"toolName":tool}, "requiredPermissions":[]}});
        let grants::Scopeable::Grant {
            covers,
            permissions,
            ..
        } = grants::scope_of(&entry)
        else {
            panic!("grant missing")
        };
        assert_eq!(permissions, [] as [vw_ask::GrantPermission; 0]);
        assert_eq!(
            covers.iter().map(|p| p.text.as_str()).collect::<String>(),
            action
        );
        assert!(covers.iter().all(|p| !p.code));
    }
}
