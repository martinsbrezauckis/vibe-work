//! Fixtures for the `vw-gate` tests: a transport that records every frame it is asked to send,
//! a scripted confirmer, a manual clock and ready-made synthetic params/callback shapes.
//!
//! The recording transport is how "nothing was sent" is proved: a blocked call must leave
//! `frames()` empty, and a forwarded call must leave exactly the frame the test expects.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vw_gate::{Clock, Confirmation, ConfirmationRequest, Confirmer, Gate, GateConfig, Transport};
use vw_rpc::{DeathCause, DeathReport, RpcError};

/// Locks a std mutex, ignoring poisoning (a panicking test already failed).
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One frame the gate handed to the transport.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub method: String,
    pub params: Value,
}

/// Records every request and answers from a script (default `{}`).
#[derive(Clone, Default)]
pub struct FakeTransport {
    frames: Arc<Mutex<Vec<Frame>>>,
    replies: Arc<Mutex<HashMap<String, Value>>>,
    dead: Arc<Mutex<bool>>,
    yield_before_record: Arc<Mutex<bool>>,
}

impl FakeTransport {
    pub fn new() -> Self {
        Self::default()
    }
    /// Every frame sent so far, oldest first.
    pub fn frames(&self) -> Vec<Frame> {
        lock(&self.frames).clone()
    }
    /// Scripts the result for `method`.
    pub fn reply(&self, method: &str, result: Value) {
        lock(&self.replies).insert(method.to_owned(), result);
    }
    /// Makes every later request fail as "server dead".
    pub fn kill(&self) {
        *lock(&self.dead) = true;
    }
    /// Makes the transport suspend before recording, so concurrent callers interleave.
    pub fn interleave(&self) {
        *lock(&self.yield_before_record) = true;
    }
}

impl Transport for FakeTransport {
    fn request(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, RpcError>> + Send {
        let this = self.clone();
        let method = method.to_owned();
        async move {
            if *lock(&this.yield_before_record) {
                for _ in 0..4 {
                    tokio::task::yield_now().await;
                }
            }
            if *lock(&this.dead) {
                return Err(RpcError::ServerDead(Box::new(DeathReport {
                    cause: DeathCause::Eof,
                    exit: None,
                    stderr_tail: Vec::new(),
                })));
            }
            lock(&this.frames).push(Frame {
                method: method.clone(),
                params,
            });
            Ok(lock(&this.replies)
                .get(&method)
                .cloned()
                .unwrap_or_else(|| json!({})))
        }
    }
}

/// Confirmer with a queue of answers (default: approve) that records what it was shown. `hold` makes the next
/// dialog wait (the fixture: one session waiting on a confirmation must not slow the others).
#[derive(Clone, Default)]
pub struct FakeConfirmer {
    answers: Arc<Mutex<VecDeque<Confirmation>>>,
    shown: Arc<Mutex<Vec<ConfirmationRequest>>>,
    holds: Arc<Mutex<VecDeque<Arc<tokio::sync::Notify>>>>,
}

impl FakeConfirmer {
    pub fn new() -> Self {
        Self::default()
    }
    /// Queues the next answer.
    pub fn then(&self, answer: Confirmation) {
        lock(&self.answers).push_back(answer);
    }
    /// Drops the queued answers that no dialog used (the fixture tests queue a decline to prove the Debug API
    /// consults no dialog; the unused decline must not answer a later window dialog).
    pub fn forget(&self) {
        lock(&self.answers).clear();
    }
    /// Everything shown to the user so far.
    pub fn shown(&self) -> Vec<ConfirmationRequest> {
        lock(&self.shown).clone()
    }
    /// The next dialog stays open until the returned handle is notified (then it is answered as queued).
    pub fn hold(&self) -> Arc<tokio::sync::Notify> {
        let notify = Arc::new(tokio::sync::Notify::new());
        lock(&self.holds).push_back(notify.clone());
        notify
    }
    /// Waits until more than `before` dialogs were shown: the core has recorded what it asks about, so a change
    /// made now happens while that dialog is open (a fixed sleep loses this race on a loaded machine).
    pub async fn wait_shown(&self, before: usize) {
        let until = Instant::now() + Duration::from_secs(30);
        while lock(&self.shown).len() <= before {
            assert!(Instant::now() < until, "no dialog within 30 s");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Confirmer for FakeConfirmer {
    fn confirm(&self, request: &ConfirmationRequest) -> impl Future<Output = Confirmation> + Send {
        lock(&self.shown).push(request.clone());
        let answer = lock(&self.answers)
            .pop_front()
            .unwrap_or(Confirmation::Approved);
        let hold = lock(&self.holds).pop_front();
        async move {
            if let Some(hold) = hold {
                hold.notified().await;
            }
            answer
        }
    }
}

/// A clock the test moves by hand.
#[derive(Clone)]
pub struct ManualClock(Arc<Mutex<Instant>>);

impl ManualClock {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }
    pub fn advance(&self, by: Duration) {
        *lock(&self.0) += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *lock(&self.0)
    }
}

/// A gate wired to the fakes, with the handles a test needs.
pub struct Rig {
    pub gate: Gate<FakeTransport, FakeConfirmer>,
    pub transport: FakeTransport,
    pub confirmer: FakeConfirmer,
    pub clock: ManualClock,
}

/// Default rig: 60 s token lifetime, up to 64 tokens.
pub fn rig() -> Rig {
    rig_with(|_| {})
}

/// Rig with the config adjusted by `tweak` (the manual clock is already installed).
pub fn rig_with(tweak: impl FnOnce(&mut GateConfig)) -> Rig {
    let transport = FakeTransport::new();
    let confirmer = FakeConfirmer::new();
    let clock = ManualClock::new();
    let mut config = GateConfig::with_clock(Arc::new(clock.clone()));
    tweak(&mut config);
    let gate = Gate::new(transport.clone(), confirmer.clone(), config);
    Rig {
        gate,
        transport,
        confirmer,
        clock,
    }
}

pub const SID: &str = "aab6f33f-55b9-dbbe-e0fb-8776689cb178";

/// `callback/call` params for a synthetic approval card.
pub fn approval_call(id: &str, session: &str) -> Value {
    json!({"callback": {
        "id": id, "sessionId": session,
        "turnId": "turn-21e7fd50a954fe54a03734ec2582066d",
        "inputEntryId": "user-turn-21e7fd50a954fe54a03734ec2582066d-1",
        "createdAt": 1_791_426_044_697_i64, "updatedAt": 1_791_426_044_697_i64,
        "generationStatus": "in_progress",
        "relatedEntryId": "effect-f9f74fa4",
        "type": "callback", "callbackId": id,
        "title": "Approve file_system.write_file",
        "detail": {"kind": "approval", "effect": {"toolName": "file_system.write_file"},
            "requiredPermissions": [],
            "choices": ["approve", "approve_for_session", "approve_permanently", "deny", "cancel_turn"],
            "pathScopeChoices": [], "relatedEntryId": "effect-f9f74fa4", "reason": null},
        "state": {"status": "open"}}})
}

/// `callback/call` params for a synthetic question card.
pub fn user_input_call(id: &str, session: &str) -> Value {
    json!({"callback": {
        "id": id, "sessionId": session, "turnId": "turn-a3036b2b",
        "createdAt": 1_791_425_621_013_i64, "updatedAt": 1_791_425_621_013_i64,
        "generationStatus": "in_progress", "type": "callback", "callbackId": id,
        "title": "User input required",
        "detail": {"kind": "user_input", "request": {"questions": [
            {"question": "Which colour do you prefer?", "header": "",
             "options": [{"label": "Red", "description": ""}, {"label": "Blue", "description": ""}],
             "multiSelect": false, "hideOther": false},
            {"question": "Which size?", "header": "", "options": [], "multiSelect": false}],
            "footerNote": null}},
        "state": {"status": "open"}}})
}

/// `callback/call` params for a callback of an arbitrary kind.
pub fn other_call(id: &str, session: &str, kind: &str) -> Value {
    json!({"callback": {
        "id": id, "sessionId": session, "type": "callback", "callbackId": id,
        "title": "x", "createdAt": 1, "updatedAt": 1, "generationStatus": "in_progress",
        "detail": {"kind": kind}, "state": {"status": "open"}}})
}

/// A correct approval answer.
pub fn approve() -> Value {
    json!({"type": "approval", "decision": {"type": "approve"}})
}

/// A `RuntimeReadResponse`-shaped value carrying the given model aliases.
pub fn runtime_with_models(aliases: &[&str]) -> Value {
    let models: Vec<Value> = aliases
        .iter()
        .map(|a| {
            json!({"name": format!("name-{a}"), "alias": a, "thinking": "off",
            "supportsImages": true, "displayName": a})
        })
        .collect();
    json!({"runtime": {"config": {"models": models}}, "ready": true})
}

/// A directory under cargo's per-target tmp dir, empty and fresh.
pub fn scratch(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("vw-gate")
        .join(format!(
            "{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Spellings of the method name `base` that must never resolve to it (case, slashes, whitespace,
/// NUL, traversal, look-alike unicode, percent-encoding). Used by synthetic validation tests.
pub fn method_variants(base: &str) -> Vec<String> {
    let mut out = vec![
        base.to_uppercase(),
        {
            let mut c = base.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        },
        format!("{base}/"),
        format!("/{base}"),
        format!(" {base}"),
        format!("{base} "),
        format!("{base}\t"),
        format!("{base}\n"),
        format!("{base}\0"),
        format!("\u{feff}{base}"),
        format!("../{base}"),
        format!("{base}/.."),
        format!("./{base}"),
        base.replacen('/', "//", 1),
        base.replacen('/', "/./", 1),
        base.replacen('/', "\\", 1),
        base.replacen('/', "\u{ff0f}", 1),  // fullwidth solidus
        base.replacen('/', "\u{2215}", 1),  // division slash
        base.replacen('/', "/\u{200b}", 1), // zero-width space
        base.replacen('o', "\u{043e}", 1),  // Cyrillic small o
        base.replacen('e', "\u{0435}", 1),  // Cyrillic small ie
        base.replacen('s', "\u{0455}", 1),  // Cyrillic dze
        format!("{base}\u{0301}"),          // trailing combining accent
        base.replacen('/', "%2F", 1),
        base.replace('/', "."),
        base.replace('/', ":"),
    ];
    out.retain(|v| v != base);
    out
}
