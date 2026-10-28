//! Where a host NAME points, for the navigation hooks and the request barrier (following ShellX's
//! baseline `browser_host_resolves_to_private_network`): a name is refused unless every answer is public, failing
//! closed on errors and timeouts. The engine's callbacks run on its UI thread, so a lookup NEVER blocks there for
//! long (a hanging resolver can freeze the window): [`NameCheck::quick`] answers from the cache or
//! within [`UI_WAIT`], else "pending" while the lookup goes on in the background; [`NameCheck::when_done`] calls back
//! when it ends (the Windows barrier holds the request with a `WebView2` deferral until then); the core waits fully
//! with [`NameCheck::resolve`] before an open, which fills the cache the hooks then read.
//!
//! Depends on: vw-netguard (address classes). Called by: `hooks`, `service` (`check_name`), tests (scripted
//! resolver).
//! Invariants:
//! - A lookup takes at most [`LOOKUP_LIMIT`] (a helper thread; a resolver that hangs is abandoned) and every
//!   registered callback is called exactly once when it ends.
//! - One lookup per name at a time (callers join the one in flight).
//! - Answers are reused for [`TTL`]; failed or timed-out lookups only for [`RETRY_AFTER`]. At most [`CACHE_CAP`]
//!   names are cached (oldest dropped).
//! - Bounded under a flood of unique names: at most [`MAX_LOOKUPS`] resolver threads live
//!   (a hung one keeps its slot, so the bound fails closed), at most [`MAX_QUEUED`] names waiting for a slot, at most
//!   [`MAX_WAITERS`] callbacks per name; anything beyond is refused at once without caching. A thread that cannot be
//!   created refuses too.
//! - Every waiter is answered within about [`LOOKUP_LIMIT`] of asking: a running lookup is abandoned
//!   at the limit; a queued name is refused at its deadline by the keeper thread even when every slot is held by a
//!   hung resolver, so a later name is never starved (it is answered or refused with the reason). Answers for a mount
//!   that closed meanwhile go to its closed webview's held request (dropped with it) or are recorded nowhere.

use std::collections::HashMap;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use vw_netguard::ip::{IpClass, classify};

/// Longest wait for one lookup (then it counts as failed: refused).
pub const LOOKUP_LIMIT: Duration = Duration::from_millis(1500);
/// Longest wait on the engine's UI thread before answering "pending".
pub const UI_WAIT: Duration = Duration::from_millis(5);
/// How long an answer is reused.
pub const TTL: Duration = Duration::from_secs(60);
/// How long a failed or timed-out lookup is reused before a new try.
pub const RETRY_AFTER: Duration = Duration::from_secs(5);
/// Names cached at most.
pub const CACHE_CAP: usize = 512;
/// Lookups running at once, counting resolver threads still stuck after [`LOOKUP_LIMIT`] (a
/// page can ask for unlimited random names). Beyond it a new name waits in a bounded queue ([`MAX_QUEUED`]).
pub const MAX_LOOKUPS: usize = 32;
/// Callers waiting for one name's lookup at most; more are refused at once.
pub const MAX_WAITERS: usize = 256;
/// Names waiting for a lookup slot at most (a busy page's many hosts wait their turn instead of being refused); a
/// queued name that waited longer than [`LOOKUP_LIMIT`] is refused when its turn comes.
pub const MAX_QUEUED: usize = 1024;

/// Resolves a name to its addresses (the system's resolver in the app; scripted in tests).
pub type NameResolver = Arc<dyn Fn(&str) -> Result<Vec<IpAddr>, String> + Send + Sync>;

/// A callback for the end of a lookup.
pub type NameDone = Box<dyn FnOnce(&NameVerdict) + Send>;

/// Where a name points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameVerdict {
    /// Every answer is public.
    Public,
    /// Some answers are private-network or carrier-grade-NAT addresses, the rest public (a page the USER may open).
    LocalNetwork(String),
    /// Refused for everyone: a loopback, link-local or special answer, no answer, or a failed or slow lookup.
    Refused(String),
}

impl NameVerdict {
    /// The refusal text for a page that may reach public names only (`None`: public).
    #[must_use]
    pub fn refusal(&self) -> Option<&str> {
        match self {
            Self::Public => None,
            Self::LocalNetwork(why) | Self::Refused(why) => Some(why),
        }
    }

    /// The refusal text for a page that may reach public names and, when `own_lan_host` (the name is the host of the
    /// Lan page the user opened), local-network answers too. Loopback, link-local and special answers stay refused
    /// for every page (the Lan page's own name is looked up like any other, so a name
    /// that points, or later rebinds, to this machine is refused instead of exempted).
    #[must_use]
    pub fn refusal_on(&self, own_lan_host: bool) -> Option<&str> {
        match self {
            Self::LocalNetwork(_) if own_lan_host => None,
            other => other.refusal(),
        }
    }

    fn transient(&self) -> bool {
        matches!(self, Self::Refused(why) if why.starts_with("could not check"))
    }
}

/// The system resolver (`getaddrinfo`).
#[must_use]
pub fn system_resolver() -> NameResolver {
    Arc::new(|host| {
        (host, 0)
            .to_socket_addrs()
            .map(|addrs| addrs.map(|a| a.ip()).collect())
            .map_err(|e| e.to_string())
    })
}

/// The verdict for one name's answers.
fn judge(host: &str, answers: Result<Vec<IpAddr>, String>) -> NameVerdict {
    let addrs = match answers {
        Ok(addrs) if addrs.is_empty() => {
            return NameVerdict::Refused(format!("{host} has no address"));
        }
        Ok(addrs) => addrs,
        Err(_) => {
            return NameVerdict::Refused(format!("could not check where {host} points; try again"));
        }
    };
    let mut verdict = NameVerdict::Public;
    for ip in addrs {
        match classify(ip) {
            IpClass::Public => {}
            IpClass::Private | IpClass::SharedCgnat => {
                verdict = NameVerdict::LocalNetwork(format!(
                    "{host} resolves to {ip}, {}",
                    classify(ip).describe()
                ));
            }
            class => {
                return NameVerdict::Refused(format!(
                    "{host} resolves to {ip}, {}",
                    class.describe()
                ));
            }
        }
    }
    verdict
}

#[derive(Default)]
struct Shared {
    cache: HashMap<String, (Instant, NameVerdict)>,
    inflight: HashMap<String, Vec<NameDone>>,
    /// Resolver threads alive (finished ones leave; a hung one stays until its resolver returns).
    live: usize,
    /// Names waiting for a slot, oldest first, with when they were queued.
    queue: std::collections::VecDeque<(String, Instant)>,
    /// The keeper thread that refuses queued names past their deadline runs (at most one).
    keeper: bool,
}

/// How often the keeper looks at the queue.
const KEEPER_TICK: Duration = Duration::from_millis(100);

/// The refusal when the bounds are reached (never cached: the next try may get a slot).
fn busy(host: &str) -> NameVerdict {
    NameVerdict::Refused(format!(
        "could not check where {host} points: too many names are being checked right now; try again"
    ))
}

/// Sizes of the name check, for tests and diagnostics: cached names, names being looked up, resolver threads alive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NameStats {
    /// Cached names (at most [`CACHE_CAP`]).
    pub cached: usize,
    /// Names with a lookup in flight or queued.
    pub inflight: usize,
    /// Names waiting for a slot (at most [`MAX_QUEUED`]).
    pub queued: usize,
    /// Resolver threads alive (at most [`MAX_LOOKUPS`]).
    pub live: usize,
}

/// A cached, bounded, non-blocking name check (cheap to clone).
#[derive(Clone)]
pub(crate) struct NameCheck {
    resolver: Arc<Mutex<NameResolver>>,
    shared: Arc<Mutex<Shared>>,
}

impl NameCheck {
    pub(crate) fn new() -> Self {
        Self {
            resolver: Arc::new(Mutex::new(system_resolver())),
            shared: Arc::new(Mutex::new(Shared::default())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The resolver in use (for a lookup that needs the addresses themselves: a capture's Lan media).
    pub(crate) fn resolver(&self) -> NameResolver {
        self.resolver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the resolver (and forgets cached answers).
    pub(crate) fn set_resolver(&self, resolver: NameResolver) {
        *self.resolver.lock().unwrap_or_else(PoisonError::into_inner) = resolver;
        self.lock().cache.clear();
    }

    fn cached(&self, host: &str) -> Option<NameVerdict> {
        let shared = self.lock();
        let (at, verdict) = shared.cache.get(host)?;
        let life = if verdict.transient() {
            RETRY_AFTER
        } else {
            TTL
        };
        (at.elapsed() < life).then(|| verdict.clone())
    }

    /// Calls `done` with the verdict for `host`: at once (on this thread) when cached, else when the lookup ends (on
    /// a helper thread), starting one unless it is in flight.
    pub(crate) fn when_done(&self, host: &str, done: NameDone) {
        let host = normal(host);
        if let Some(verdict) = self.cached(&host) {
            done(&verdict);
            return;
        }
        let refused = {
            let mut shared = self.lock();
            let full = shared.live >= MAX_LOOKUPS;
            let room = shared.queue.len() < MAX_QUEUED;
            match shared.inflight.get_mut(&host) {
                Some(waiting) if waiting.len() < MAX_WAITERS => {
                    waiting.push(done);
                    None
                }
                Some(_) => Some(done),
                None if full && room => {
                    shared.inflight.insert(host.clone(), vec![done]);
                    shared.queue.push_back((host.clone(), Instant::now()));
                    let start_keeper = !shared.keeper;
                    shared.keeper = true;
                    drop(shared);
                    if start_keeper {
                        self.keep();
                    }
                    None
                }
                None if full => Some(done),
                None => {
                    shared.inflight.insert(host.clone(), vec![done]);
                    shared.live += 1;
                    drop(shared);
                    self.start(&host);
                    None
                }
            }
        };
        if let Some(done) = refused {
            done(&busy(&host));
        }
    }

    /// The current sizes.
    pub(crate) fn stats(&self) -> NameStats {
        let shared = self.lock();
        NameStats {
            cached: shared.cache.len(),
            inflight: shared.inflight.len(),
            queued: shared.queue.len(),
            live: shared.live,
        }
    }

    /// The verdict when it is known within `wait` (cached, or a lookup that ends in time); `None`: still pending
    /// (the lookup goes on and fills the cache).
    pub(crate) fn quick(&self, host: &str, wait: Duration) -> Option<NameVerdict> {
        let host = normal(host);
        if let Some(verdict) = self.cached(&host) {
            return Some(verdict);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.when_done(
            &host,
            Box::new(move |verdict| {
                let _ = tx.send(verdict.clone());
            }),
        );
        rx.recv_timeout(wait).ok()
    }

    /// The verdict, waiting for the lookup (at most [`LOOKUP_LIMIT`] plus scheduling). Never call on a UI thread.
    pub(crate) fn resolve(&self, host: &str) -> NameVerdict {
        self.quick(host, LOOKUP_LIMIT + Duration::from_millis(500))
            .unwrap_or_else(|| {
                NameVerdict::Refused(format!("could not check where {host} points; try again"))
            })
    }

    /// Runs one lookup (its slot in `live` is already taken), bounded by [`LOOKUP_LIMIT`]; records and announces the
    /// verdict. The resolver runs on its own thread, so a hanging one is abandoned after the limit (it keeps its slot
    /// until it returns).
    fn start(&self, host: &str) {
        let resolver = self
            .resolver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let (name, slot) = (host.to_owned(), self.clone());
        let spawned = std::thread::Builder::new()
            .name("vw-name-lookup".to_owned())
            .spawn(move || {
                let answers = resolver(&name);
                let _ = tx.send(answers);
                slot.lock().live -= 1;
                slot.next();
            });
        if spawned.is_err() {
            self.lock().live -= 1;
            self.finish(host, &busy(host), false);
            self.next();
            return;
        }
        let (this, waited) = (self.clone(), host.to_owned());
        let watcher = std::thread::Builder::new()
            .name("vw-name-wait".to_owned())
            .spawn(move || {
                let verdict = match rx.recv_timeout(LOOKUP_LIMIT) {
                    Ok(answers) => judge(&waited, answers),
                    Err(_) => NameVerdict::Refused(format!(
                        "could not check where {waited} points; try again"
                    )),
                };
                this.finish(&waited, &verdict, true);
            });
        if watcher.is_err() {
            // The resolver thread runs on and frees its slot; the callers get a refusal now.
            self.finish(host, &busy(host), false);
        }
    }

    /// Starts the oldest queued name when a slot is free; queued names that waited past [`LOOKUP_LIMIT`] are refused
    /// on the way (their callers have waited long enough).
    fn next(&self) {
        loop {
            let (host, expired) = {
                let mut shared = self.lock();
                if shared.live >= MAX_LOOKUPS {
                    return;
                }
                let Some((host, at)) = shared.queue.pop_front() else {
                    return;
                };
                let expired = at.elapsed() > LOOKUP_LIMIT;
                if !expired {
                    shared.live += 1;
                }
                (host, expired)
            };
            if expired {
                self.finish(&host, &busy(&host), false);
            } else {
                self.start(&host);
                return;
            }
        }
    }

    /// Runs the keeper (a queue whose slots are all held by hung resolvers must not hold its names
    /// forever): every [`KEEPER_TICK`] it refuses queued names that waited past [`LOOKUP_LIMIT`], and ends when the
    /// queue is empty. One thread at most; if it cannot be created, queued names expire when a slot frees (`next`).
    fn keep(&self) {
        let this = self.clone();
        let spawned = std::thread::Builder::new()
            .name("vw-name-queue".to_owned())
            .spawn(move || {
                loop {
                    std::thread::sleep(KEEPER_TICK);
                    let expired: Vec<String> = {
                        let mut shared = this.lock();
                        let mut out = Vec::new();
                        while shared
                            .queue
                            .front()
                            .is_some_and(|(_, at)| at.elapsed() > LOOKUP_LIMIT)
                        {
                            if let Some((host, _)) = shared.queue.pop_front() {
                                out.push(host);
                            }
                        }
                        if shared.queue.is_empty() && out.is_empty() {
                            shared.keeper = false;
                            return;
                        }
                        out
                    };
                    for host in expired {
                        this.finish(&host, &busy(&host), false);
                    }
                }
            });
        if spawned.is_err() {
            self.lock().keeper = false;
        }
    }

    /// Records `verdict` (when `cache`) and calls everyone waiting for `host`.
    fn finish(&self, host: &str, verdict: &NameVerdict, cache: bool) {
        let waiting = {
            let mut shared = self.lock();
            if cache {
                if shared.cache.len() >= CACHE_CAP
                    && let Some(oldest) = shared
                        .cache
                        .iter()
                        .min_by_key(|(_, (at, _))| *at)
                        .map(|(k, _)| k.clone())
                {
                    shared.cache.remove(&oldest);
                }
                shared
                    .cache
                    .insert(host.to_owned(), (Instant::now(), verdict.clone()));
            }
            shared.inflight.remove(host).unwrap_or_default()
        };
        for done in waiting {
            done(verdict);
        }
    }
}

/// Lowercase, without a trailing dot.
fn normal(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}
