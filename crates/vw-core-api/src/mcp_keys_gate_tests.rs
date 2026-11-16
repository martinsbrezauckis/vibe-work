//! The gate's checks, each pinned by a vault that changes at one await point (module docs of `mcp_keys_gate`).
//!
//! PREDICT (written before the gate): a path that reads the key before checking the grant and the Never permission on
//! the record read after the consent, or that takes the post-dialog record without comparing it with the shown one,
//! fails `never_set_during_the_dialog_reads_no_key`, `a_moved_record_after_the_dialog_reads_no_key` and
//! `a_yes_without_a_grant_reads_no_key` (the value is read, or the moved record is used). Every other test pins one
//! check: a gate that leaves that check out fails it.

use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use vw_keyproxy::McpRefusal;
use vw_keys::McpBinding;
use vw_vault::SecretString;

use super::{Gate, Vault};
use crate::mcp_keys::McpKeysState;

const ID: &str = "binding-1";

/// A change the fake vault applies right after one kind of read.
type Change = Box<dyn FnOnce(&mut State) + Send>;

#[derive(Default)]
struct State {
    record: Option<McpBinding>,
    never: bool,
    record_reads: u32,
    never_reads: u32,
    value_reads: u32,
    /// Applied after the next value read (a change while the key is read).
    after_value: Option<Change>,
}

#[derive(Default, Clone)]
struct Fake(Arc<Mutex<State>>);

impl Fake {
    fn with(record: McpBinding) -> Self {
        let fake = Self::default();
        fake.state().record = Some(record);
        fake
    }
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn change(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.state());
    }
}

// Plain functions returning ready futures: the fake never waits, so a change happens exactly between two reads.
impl Vault for Fake {
    fn record(&self, id: &str) -> impl Future<Output = Result<McpBinding, McpRefusal>> + Send {
        let mut state = self.state();
        state.record_reads += 1;
        ready(
            state
                .record
                .clone()
                .filter(|b| b.id == id)
                .ok_or(McpRefusal::Unknown),
        )
    }
    fn never(&self, _secret_id: &str) -> impl Future<Output = Result<bool, McpRefusal>> + Send {
        let mut state = self.state();
        state.never_reads += 1;
        ready(Ok(state.never))
    }
    fn value(
        &self,
        _secret_id: &str,
    ) -> impl Future<Output = Result<SecretString, McpRefusal>> + Send {
        let mut state = self.state();
        state.value_reads += 1;
        if let Some(change) = state.after_value.take() {
            change(&mut state);
        }
        ready(Ok(SecretString::new("fixture-key-not-real")))
    }
}

fn record(always: bool) -> McpBinding {
    McpBinding {
        id: ID.to_owned(),
        server: "docs".to_owned(),
        upstream: "https://docs.example.test/mcp".to_owned(),
        header: "authorization".to_owned(),
        bearer: true,
        secret_id: "secret-1".to_owned(),
        secret_name: "docs key".to_owned(),
        always,
        revoked_ms: 0,
        created_ms: 1,
    }
}

/// Runs the gate for `ID`; `ask` is the first-use dialog (it may change the vault and the grants).
async fn open(
    fake: &Fake,
    grants: &McpKeysState,
    paused: &AtomicBool,
    ask: impl FnOnce(McpBinding) -> bool,
) -> Result<McpBinding, McpRefusal> {
    let is_paused = || paused.load(Ordering::SeqCst);
    let gate = Gate {
        vault: fake,
        grants,
        paused: &is_paused,
        id: ID,
    };
    gate.open(|shown| ready(ask(shown)))
        .await
        .map(|(binding, _value)| binding)
}

fn no_dialog(_: McpBinding) -> bool {
    panic!("no dialog may open here")
}

#[tokio::test]
async fn a_standing_grant_reads_the_key_once_between_two_checked_reads() {
    let fake = Fake::with(record(true));
    let result = open(
        &fake,
        &McpKeysState::default(),
        &AtomicBool::new(false),
        no_dialog,
    )
    .await;
    assert_eq!(result.map(|b| b.upstream), Ok(record(true).upstream));
    let state = fake.state();
    assert_eq!(
        (state.record_reads, state.never_reads, state.value_reads),
        (2, 2, 1)
    );
}

#[tokio::test]
async fn a_paused_agent_reads_nothing() {
    let fake = Fake::with(record(true));
    let result = open(
        &fake,
        &McpKeysState::default(),
        &AtomicBool::new(true),
        no_dialog,
    )
    .await;
    assert_eq!(result, Err(McpRefusal::AgentAccessLocked));
    assert_eq!(fake.state().record_reads, 0);
    assert_eq!(fake.state().value_reads, 0);
}

#[tokio::test]
async fn the_debug_bearer_is_refused_even_with_a_grant() {
    let mut debug = record(true);
    debug.secret_id = vw_keys::DEBUG_TOKEN_ITEM.to_owned();
    let fake = Fake::with(debug);
    let result = open(
        &fake,
        &McpKeysState::default(),
        &AtomicBool::new(false),
        no_dialog,
    )
    .await;
    assert_eq!(result, Err(McpRefusal::DebugTokenRequiresChat));
    assert_eq!(fake.state().value_reads, 0);
}

#[tokio::test]
async fn never_refuses_a_granted_record_and_never_opens_a_dialog() {
    for always in [true, false] {
        let fake = Fake::with(record(always));
        fake.change(|s| s.never = true);
        let result = open(
            &fake,
            &McpKeysState::default(),
            &AtomicBool::new(false),
            no_dialog,
        )
        .await;
        assert_eq!(result, Err(McpRefusal::NotAllowed), "always: {always}");
        assert_eq!(fake.state().value_reads, 0);
    }
}

#[tokio::test]
async fn no_grant_and_no_yes_reads_no_key() {
    let fake = Fake::with(record(false));
    let mut asked = None;
    let result = open(
        &fake,
        &McpKeysState::default(),
        &AtomicBool::new(false),
        |shown| {
            asked = Some(shown);
            false
        },
    )
    .await;
    assert_eq!(result, Err(McpRefusal::NotAllowed));
    assert_eq!(
        asked,
        Some(record(false)),
        "the dialog gets the checked record"
    );
    assert_eq!(fake.state().value_reads, 0);
}

#[tokio::test]
async fn a_yes_without_a_grant_reads_no_key() {
    // The dialog's task reports a grant, but none is on the record or in this run's grants.
    let fake = Fake::with(record(false));
    let result = open(
        &fake,
        &McpKeysState::default(),
        &AtomicBool::new(false),
        |_| true,
    )
    .await;
    assert_eq!(result, Err(McpRefusal::NotAllowed));
    assert_eq!(fake.state().value_reads, 0);
}

#[tokio::test]
async fn a_granted_answer_reads_the_key_after_a_fresh_check() {
    for always in [true, false] {
        let fake = Fake::with(record(false));
        let grants = McpKeysState::default();
        let result = open(&fake, &grants, &AtomicBool::new(false), |shown| {
            if always {
                fake.change(|s| s.record.as_mut().unwrap().always = true);
            } else {
                grants.give_plain(&shown);
            }
            true
        })
        .await;
        assert!(result.is_ok(), "always: {always}: {result:?}");
        let state = fake.state();
        assert_eq!((state.record_reads, state.value_reads), (3, 1));
    }
}

#[tokio::test]
async fn never_set_during_the_dialog_reads_no_key() {
    let fake = Fake::with(record(false));
    let grants = McpKeysState::default();
    let result = open(&fake, &grants, &AtomicBool::new(false), |shown| {
        grants.give_plain(&shown);
        fake.change(|s| s.never = true);
        true
    })
    .await;
    assert_eq!(result, Err(McpRefusal::NotAllowed));
    assert_eq!(fake.state().value_reads, 0, "no key read after a Never");
}

#[tokio::test]
async fn a_revoke_during_the_dialog_reads_no_key() {
    let fake = Fake::with(record(false));
    let grants = McpKeysState::default();
    let result = open(&fake, &grants, &AtomicBool::new(false), |shown| {
        grants.give_plain(&shown);
        fake.change(|s| s.record.as_mut().unwrap().revoked_ms = 5);
        true
    })
    .await;
    assert_eq!(result, Err(McpRefusal::NotAllowed));
    assert_eq!(fake.state().value_reads, 0);
}

#[tokio::test]
async fn a_moved_record_after_the_dialog_reads_no_key() {
    // "Always" lands on the record, which another process has pointed elsewhere meanwhile.
    let fake = Fake::with(record(false));
    let result = open(
        &fake,
        &McpKeysState::default(),
        &AtomicBool::new(false),
        |_| {
            fake.change(|s| {
                let r = s.record.as_mut().unwrap();
                r.always = true;
                r.upstream = "https://elsewhere.example.test/mcp".to_owned();
            });
            true
        },
    )
    .await;
    assert_eq!(result, Err(McpRefusal::NotAllowed));
    assert_eq!(fake.state().value_reads, 0);
}

#[tokio::test]
async fn a_plain_grant_covers_only_the_record_it_was_given_for() {
    let fake = Fake::with(record(false));
    let grants = McpKeysState::default();
    grants.give_plain(&record(false));
    fake.change(|s| s.record.as_mut().unwrap().header = "x-other".to_owned());
    let result = open(&fake, &grants, &AtomicBool::new(false), |_| false).await;
    assert_eq!(result, Err(McpRefusal::NotAllowed));
    assert_eq!(fake.state().value_reads, 0);
}

/// Each change made while the key's value is read refuses the request.
#[tokio::test]
async fn a_change_while_the_key_is_read_refuses() {
    let paused = Arc::new(AtomicBool::new(false));
    let cases: Vec<(&str, Change, McpRefusal)> = vec![
        (
            "revoke",
            Box::new(|s: &mut State| {
                let r = s.record.as_mut().unwrap();
                r.always = false;
                r.revoked_ms = 9;
            }),
            McpRefusal::NotAllowed,
        ),
        (
            "never",
            Box::new(|s: &mut State| s.never = true),
            McpRefusal::NotAllowed,
        ),
        (
            "moved",
            Box::new(|s: &mut State| {
                s.record.as_mut().unwrap().upstream =
                    "https://elsewhere.example.test/mcp".to_owned();
            }),
            McpRefusal::NotAllowed,
        ),
        (
            "removed",
            Box::new(|s: &mut State| s.record = None),
            McpRefusal::Unknown,
        ),
        (
            "paused",
            Box::new({
                let paused = Arc::clone(&paused);
                move |_: &mut State| paused.store(true, Ordering::SeqCst)
            }),
            McpRefusal::AgentAccessLocked,
        ),
    ];
    for (name, change, refusal) in cases {
        paused.store(false, Ordering::SeqCst);
        let fake = Fake::with(record(true));
        fake.change(|s| s.after_value = Some(change));
        let result = open(&fake, &McpKeysState::default(), &paused, no_dialog).await;
        assert_eq!(result, Err(refusal), "{name}");
        assert_eq!(fake.state().value_reads, 1, "{name}");
    }
}
