//! In-memory ask registry with no answer timeout. Native presenters and card adapters share settlement.
//! Depends on the injected clock, broadcast events and watch replies; no request parameters are retained.
use crate::CoreEvent;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::{broadcast, watch};
use vw_ask::{AnsweredBy, Ask, AskRecord, Choice, Outcome, Scope, Surface};

#[path = "delivery.rs"]
mod delivery;

/// Clock shared by every ask; tests may advance it to verify waiting cards.
pub trait AskClock: Send + Sync {
    /// Unix milliseconds, advancing monotonically for this run.
    fn now_ms(&self) -> u64;
}
/// Production timestamps follow wall and monotonic time without running backwards.
pub struct SystemAskClock {
    wall: u64,
    instant: tokio::time::Instant,
    now_wall: fn() -> u64,
    last: std::sync::atomic::AtomicU64,
}
fn wall_now() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
impl Default for SystemAskClock {
    fn default() -> Self {
        Self::with_wall(wall_now)
    }
}
impl SystemAskClock {
    /// The production clock with a replaceable wall-time source (tests simulate a machine sleep with it).
    #[must_use]
    pub fn with_wall(now_wall: fn() -> u64) -> Self {
        Self {
            wall: now_wall(),
            instant: tokio::time::Instant::now(),
            now_wall,
            last: std::sync::atomic::AtomicU64::new(0),
        }
    }
}
impl AskClock for SystemAskClock {
    fn now_ms(&self) -> u64 {
        let monotonic = self
            .wall
            .saturating_add(u64::try_from(self.instant.elapsed().as_millis()).unwrap_or(u64::MAX));
        let now = monotonic.max((self.now_wall)());
        self.last
            .fetch_max(now, std::sync::atomic::Ordering::SeqCst)
            .max(now)
    }
}
/// How [`Service::open_with`] treats an `external_id` that is already open, and the ask's `every_call` flag.
#[derive(Clone, Copy)]
struct How {
    join: bool,
    every_call: bool,
}
impl How {
    const JOIN: Self = Self {
        join: true,
        every_call: false,
    };
}
struct Entry {
    ask: Ask,
    reply: watch::Sender<Option<Outcome>>,
    delivering: bool,
}
struct State {
    next: u64,
    open: BTreeMap<String, Entry>,
    records: VecDeque<AskRecord>,
}
pub(crate) struct Service {
    state: Mutex<State>,
    clock: Arc<dyn AskClock>,
    events: broadcast::Sender<CoreEvent>,
}
impl Service {
    pub(crate) fn new(clock: Arc<dyn AskClock>, events: broadcast::Sender<CoreEvent>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                next: 0,
                open: BTreeMap::new(),
                records: VecDeque::new(),
            }),
            clock,
            events,
        })
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// The ask clock's current time (Unix milliseconds), also used to stamp session grants.
    pub(crate) fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }
    /// Opens an ask, or joins the open ask with `external_id` (a callback or proposal registered again).
    pub(crate) fn open(
        self: &Arc<Self>,
        kind: &str,
        session: Option<String>,
        requester: impl Into<vw_ask::Requester>,
        offers: Vec<Choice>,
        external_id: Option<String>,
    ) -> Handle {
        self.open_with(
            kind,
            session,
            requester.into(),
            offers,
            external_id,
            How::JOIN,
        )
    }
    /// [`Self::open`] for a Vibe approval whose "Allow for this session" would cover every call of its tool
    /// (`every_call`, shown on the card before the click).
    pub(crate) fn open_callback(
        self: &Arc<Self>,
        kind: &str,
        session: Option<String>,
        offers: Vec<Choice>,
        external_id: Option<String>,
        every_call: bool,
    ) -> Handle {
        self.open_with(
            kind,
            session,
            vw_ask::Requester::Agent,
            offers,
            external_id,
            How {
                join: true,
                every_call,
            },
        )
    }
    /// Opens a new ask for one native request. It never joins: an `external_id` that is already open or recorded
    /// is replaced by a fresh run-local id, so a request that names another ask's id gets its own answer instead
    /// of riding on that ask's yes.
    pub(crate) fn open_own(
        self: &Arc<Self>,
        kind: &str,
        session: Option<String>,
        requester: vw_ask::Requester,
        offers: Vec<Choice>,
        external_id: Option<String>,
    ) -> Handle {
        self.open_with(
            kind,
            session,
            requester,
            offers,
            external_id,
            How {
                join: false,
                every_call: false,
            },
        )
    }
    fn open_with(
        self: &Arc<Self>,
        kind: &str,
        session: Option<String>,
        requester: vw_ask::Requester,
        offers: Vec<Choice>,
        external_id: Option<String>,
        how: How,
    ) -> Handle {
        let How { join, every_call } = how;
        let mut state = self.lock();
        if let Some(id) = &external_id
            && let Some(entry) = state.open.get(id)
            && join
        {
            return Handle {
                service: self.clone(),
                ask: entry.ask.clone(),
                reply: entry.reply.subscribe(),
                cancel_on_drop: false,
                opened: false,
            };
        }
        let taken = |id: &String| {
            state.open.contains_key(id) || state.records.iter().any(|r| r.ask.id == *id)
        };
        let external_id = external_id.filter(|id| join || !taken(id));
        state.next += 1;
        let policy = vw_ask::policy(kind);
        let now = self.clock.now_ms();
        let ask = Ask {
            id: external_id.unwrap_or_else(|| format!("ask-{}", state.next)),
            kind: kind.into(),
            session,
            requester,
            surface: policy.map_or(Surface::Native, |p| p.surface),
            title: policy.map_or("this request", |p| p.title).into(),
            opened_at: now,

            offers,
            person_only: policy.is_none_or(|p| p.person_only),
            every_call,
            always_setting: policy.and_then(|p| p.always_setting).map(str::to_owned),
        };
        let (send, reply) = watch::channel(None);
        state.open.insert(
            ask.id.clone(),
            Entry {
                ask: ask.clone(),
                reply: send,
                delivering: false,
            },
        );
        let _ = self.events.send(CoreEvent::AskOpened { ask: ask.clone() });
        drop(state);
        Handle {
            service: self.clone(),
            ask,
            reply,
            cancel_on_drop: false,
            opened: true,
        }
    }
    /// Records an explicit "Never" the user set for exactly this thing: no prompt opens, but
    /// the refusal stays visible as a record with `Outcome::Never` and an `AskClosed` event (the bell's Security
    /// group). Callers use it only for a user's own Never, never for a level or a default.
    pub(crate) fn record_never(&self, kind: &str, session: Option<String>) -> AskRecord {
        let mut state = self.lock();
        state.next += 1;
        let policy = vw_ask::policy(kind);
        let now = self.clock.now_ms();
        let record = AskRecord {
            ask: Ask {
                id: format!("ask-{}", state.next),
                kind: kind.into(),
                session,
                requester: vw_ask::Requester::Agent,
                surface: policy.map_or(Surface::Native, |p| p.surface),
                title: policy.map_or("this request", |p| p.title).into(),
                opened_at: now,

                offers: Vec::new(),
                person_only: policy.is_none_or(|p| p.person_only),
                every_call: false,
                always_setting: None,
            },
            closed_at: now,
            outcome: Outcome::Never,
        };
        state.records.push_back(record.clone());
        while state.records.len() > 500 {
            state.records.pop_front();
        }
        let _ = self.events.send(CoreEvent::AskClosed {
            record: record.clone(),
        });
        record
    }
    pub(crate) fn list(&self) -> Vec<Ask> {
        self.lock().open.values().map(|e| e.ask.clone()).collect()
    }
    pub(crate) fn records(&self) -> Vec<AskRecord> {
        self.lock().records.iter().cloned().collect()
    }
    pub(crate) fn get(&self, id: &str) -> Option<Ask> {
        self.lock().open.get(id).map(|e| e.ask.clone())
    }
    pub(crate) fn outcome(&self, id: &str) -> Option<AskRecord> {
        self.lock()
            .records
            .iter()
            .rev()
            .find(|r| r.ask.id == id)
            .cloned()
    }
    /// The first answer settles the open ask.
    pub(crate) fn settle(&self, id: &str, outcome: Outcome) -> bool {
        self.settle_inner(id, outcome, false)
    }
    /// Reserved answers finish from the transport result, even if a close arrived during delivery.
    fn settle_inner(&self, id: &str, outcome: Outcome, delivering: bool) -> bool {
        let mut state = self.lock();
        if state
            .open
            .get(id)
            .is_none_or(|e| e.delivering != delivering)
        {
            return false;
        }
        let Some(entry) = state.open.remove(id) else {
            return false;
        };
        let now = self.clock.now_ms();
        let record = AskRecord {
            ask: entry.ask,
            closed_at: now,
            outcome: outcome.clone(),
        };
        state.records.push_back(record.clone());
        while state.records.len() > 500 {
            state.records.pop_front();
        }
        entry.reply.send_replace(Some(outcome));
        let _ = self.events.send(CoreEvent::AskClosed { record });
        drop(state);
        true
    }
    pub(crate) fn cancel_session(&self, session: Option<&str>, reason: &str) {
        for ask in self
            .list()
            .into_iter()
            .filter(|a| session.is_none_or(|s| a.session.as_deref() == Some(s)))
        {
            self.settle(
                &ask.id,
                Outcome::Cancelled {
                    reason: reason.into(),
                },
            );
        }
    }
    pub(crate) fn replay(&self) -> Vec<CoreEvent> {
        let state = self.lock();
        state
            .records
            .iter()
            .map(|r| CoreEvent::AskClosed { record: r.clone() })
            .chain(
                state
                    .open
                    .values()
                    .map(|e| CoreEvent::AskOpened { ask: e.ask.clone() }),
            )
            .collect()
    }
}
pub(crate) struct Handle {
    service: Arc<Service>,
    pub(crate) ask: Ask,
    reply: watch::Receiver<Option<Outcome>>,
    pub(crate) cancel_on_drop: bool,
    /// This handle opened the card; joined handles must not schedule another settlement side effect.
    pub(crate) opened: bool,
}
impl Handle {
    pub(crate) async fn closed(&mut self) -> Outcome {
        loop {
            if let Some(outcome) = self.reply.borrow_and_update().clone() {
                return outcome;
            }
            if self.reply.changed().await.is_err() {
                return Outcome::Cancelled {
                    reason: "Vibe Work closed".into(),
                };
            }
        }
    }
    pub(crate) fn finish(&self, outcome: Outcome) {
        self.service.settle(&self.ask.id, outcome);
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.finish(Outcome::Cancelled {
                reason: "the caller disconnected".into(),
            });
        }
    }
}
pub(crate) fn allowed(by: AnsweredBy, scope: Scope) -> Outcome {
    Outcome::Allowed { scope, by }
}
pub(crate) fn refused(by: AnsweredBy, note: Option<String>) -> Outcome {
    Outcome::Refused {
        by,
        note: note.map(|n| scrub_note(&n)),
    }
}

/// Scrubs registered secret values first, then trims and limits: limiting first could cut a secret so that its
/// start no longer matches and stays in the note.
pub(crate) fn scrub_note(note: &str) -> String {
    let scrubbed = vw_secrets::scrub(note.as_bytes()).map_or_else(
        || note.to_owned(),
        |(bytes, _)| String::from_utf8_lossy(&bytes).into_owned(),
    );
    vw_ask::plain_note(&scrubbed)
}
