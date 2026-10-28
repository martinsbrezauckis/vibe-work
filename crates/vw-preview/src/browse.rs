//! What the agent browser reads from a session's preview besides the view: the page environment's
//! accessor, the load counter (receipts wait for a navigation an action started), refusals with their reasons, and
//! the loopback gate the network guard asks for a mount's requests to this computer.
//!
//! Depends on: `service` (the registry), `engine` (mount identity), `policy`. Called by: vw-browser
//! (driver), vw-core-api (the guard's page context), tests.
//! Invariants: read-only views of the registry except [`PreviewService::loopback_gate`], which consumes nothing
//! either (a navigation allowance expires by time).

use crate::console::{ConsoleEntry, MAX_RETURNED};
use crate::engine::MountId;
use crate::policy::PageClass;
use crate::service::PreviewError;
use crate::service::PreviewService;

/// A snapshot of a session's loading state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadState {
    /// Finished loads so far (moves with every load).
    pub loads: u64,
    /// A load is in progress.
    pub loading: bool,
    /// The current URL.
    pub url: Option<String>,
}

/// The first answer for a request of a webview mount to `127.0.0.1:<port>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoopbackGate {
    /// The mount holds a page of this session's own server (the session id): the port's owner decides (own or
    /// another session's server).
    NeedOwner(String),
    /// Refused: the reason.
    Refuse(String),
}

/// The mount labels whose current document holds a vault-filled password, process-wide, so the app's
/// whole-window screenshot (vw-webview `window_shot`, which pictures previews as child webviews) refuses too.
fn protected() -> std::sync::MutexGuard<'static, std::collections::HashSet<String>> {
    static LABELS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    LABELS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// True while the webview `label` shows a document a vault password was filled into.
#[must_use]
pub fn shots_protected(label: &str) -> bool {
    protected().contains(label)
}

/// The document of mount `label` was replaced: its protection ends (called where the load count moves).
pub(crate) fn unprotect(label: &str) {
    protected().remove(label);
}

/// Why a screenshot is refused after a vault fill.
pub const SHOTS_BLOCKED: &str = "screenshots of this page are off after a vault login was filled into it (the password \
    could show in the picture); ask the user to look at the page, or open another page first";

impl PreviewService {
    /// Following ShellX's protected-value rule, screenshot capture is blocked after a host-mediated
    /// secret fill: every screenshot of the session's preview is refused until it shows another document (its load
    /// count moves).
    pub fn block_shots_on_this_document(&self, session_id: &str) {
        let mut state = self.inner.lock();
        let owner = state.owner_of(session_id);
        if let Some(tabs) = state.tabs.get_mut(&owner) {
            tabs.snapshots.remove(session_id);
        }
        if let Some(entry) = state.entries.get_mut(session_id) {
            entry.shots_blocked_at = Some(*entry.loads.borrow());
            if let Some(mount) = &entry.mount {
                protected().insert(mount.label.clone());
            }
        }
    }

    /// True while [`PreviewService::block_shots_on_this_document`] holds for the session.
    #[must_use]
    pub fn shots_blocked(&self, session_id: &str) -> bool {
        let state = self.inner.lock();
        state
            .entries
            .get(session_id)
            .is_some_and(|e| e.shots_blocked_at == Some(*e.loads.borrow()))
    }

    /// The page environment's accessor of the session's current mount (`None`: not mounted).
    #[must_use]
    pub fn page_accessor(&self, session_id: &str) -> Option<String> {
        let state = self.inner.lock();
        state
            .entries
            .get(session_id)
            .and_then(crate::state::Entry::ready_mount)
            .map(|m| m.page.clone())
    }

    /// The identity of the session's current mount (`None`: not mounted).
    #[must_use]
    pub fn page_mount(&self, session_id: &str) -> Option<MountId> {
        let state = self.inner.lock();
        state
            .entries
            .get(session_id)
            .and_then(crate::state::Entry::ready_mount)
            .map(|mount| MountId {
                session_id: session_id.to_owned(),
                label: mount.label.clone(),
            })
    }

    /// The session's loading state (`None`: never opened).
    #[must_use]
    pub fn load_state(&self, session_id: &str) -> Option<LoadState> {
        let state = self.inner.lock();
        state.entries.get(session_id).map(|e| LoadState {
            loads: *e.loads.borrow(),
            loading: e.loading,
            url: e.url.as_ref().map(ToString::to_string),
        })
    }

    /// Refused navigations since the last page-diagnostics reset, oldest first (at most `MAX_REFUSED`).
    #[must_use]
    pub fn refusals(&self, session_id: &str) -> Vec<(String, String)> {
        let state = self.inner.lock();
        state
            .entries
            .get(session_id)
            .map(|e| e.refused_why.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// A lifetime mark for navigation refusals, including repeated URLs.
    #[must_use]
    pub fn refusal_mark(&self, session_id: &str) -> u64 {
        self.inner
            .lock()
            .entries
            .get(session_id)
            .map_or(0, |e| e.refusals_seen)
    }

    /// Retained refusals recorded after `mark`, oldest first.
    #[must_use]
    pub fn refusals_since(&self, session_id: &str, mark: u64) -> Vec<(String, String)> {
        let state = self.inner.lock();
        let Some(entry) = state.entries.get(session_id) else {
            return Vec::new();
        };
        let count = usize::try_from(entry.refusals_seen.saturating_sub(mark))
            .unwrap_or(usize::MAX)
            .min(entry.refused_why.len());
        entry
            .refused_why
            .iter()
            .skip(entry.refused_why.len() - count)
            .cloned()
            .collect()
    }

    /// True when the network guard is proven active for the session's current mount.
    #[must_use]
    pub fn guarded(&self, session_id: &str) -> bool {
        self.inner
            .lock()
            .entries
            .get(session_id)
            .is_some_and(|e| e.guarded && e.ready_mount().is_some())
    }

    /// The protection layers of the session's current mount besides the navigation policy, as wire
    /// names in the order the engine reported them (`requestBarrier`, `egressProxy`, `contentRules`); empty when
    /// nothing is mounted. For diagnostics and tests (the Debug API's browser answers).
    #[must_use]
    pub fn guard_layers(&self, session_id: &str) -> Vec<&'static str> {
        self.inner
            .lock()
            .entries
            .get(session_id)
            .filter(|e| e.ready_mount().is_some())
            .map(|e| e.layers.iter().map(|l| l.name()).collect())
            .unwrap_or_default()
    }

    /// Page key owning this live native mount, for guard refusal attribution.
    #[must_use]
    pub fn page_for_mount(&self, label: &str) -> Option<String> {
        self.inner.lock().session_of(label).map(str::to_owned)
    }

    /// Reports final native navigation refusals to the core without copying URLs or request values.
    pub fn on_guard_refused(&self, listener: impl Fn(&str, &str) + Send + Sync + 'static) {
        *self
            .inner
            .guard_refused
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(std::sync::Arc::new(listener));
    }

    /// The labels of every live mount.
    #[must_use]
    pub fn mount_labels(&self) -> Vec<String> {
        self.inner
            .lock()
            .entries
            .values()
            .filter_map(|e| e.mount.as_ref().map(|m| m.label.clone()))
            .collect()
    }

    /// Gives the sub-request rule the network guard's list of Vibe Work's own ports.
    pub fn set_network_guard(&self, guard: vw_netguard::NetGuard) {
        *self
            .inner
            .guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(guard);
    }

    /// The page class of the session's current mount; a suspended local-network page the user opened is still `Lan`
    /// (it mounts again as one). `None`: nothing mounted.
    #[must_use]
    pub fn page_class(&self, session_id: &str) -> Option<PageClass> {
        let state = self.inner.lock();
        let entry = state.entries.get(session_id)?;
        entry
            .mount
            .as_ref()
            .map(|m| m.class)
            .or_else(|| entry.lan_host.is_some().then_some(PageClass::Lan))
    }

    /// The loopback rule's first step for a request of the webview mounted as `label` to a port on this computer
    /// (following the Private Network Access model): a mount holding a page of the session's own server asks the
    /// port's owner next (answering the session); a mount holding a page from the web, or an unknown label, is
    /// refused. A mount's class never changes (`PageClass`), so no allowance is ever carried across a navigation.
    #[must_use]
    pub fn loopback_gate(&self, label: &str) -> LoopbackGate {
        let state = self.inner.lock();
        let Some(session) = state.session_of(label) else {
            return LoopbackGate::Refuse("this page is no longer shown".to_owned());
        };
        let class = state
            .entries
            .get(session)
            .and_then(|entry| entry.mount.as_ref())
            .filter(|m| m.label == label)
            .map(|m| m.class);
        match class {
            Some(PageClass::Local) => LoopbackGate::NeedOwner(state.owner_of(session)),
            Some(PageClass::Web) => LoopbackGate::Refuse(
                "a page from the web cannot reach servers on this computer".to_owned(),
            ),
            Some(PageClass::Lan) => LoopbackGate::Refuse(
                "a page on your local network cannot reach servers on this computer".to_owned(),
            ),
            None => LoopbackGate::Refuse("this page is no longer shown".to_owned()),
        }
    }

    /// Whether an answer of a page script built for the mount whose page accessor is `accessor` and that reports
    /// `url` may be used (to prevent stale-mount use): the session's current mount must still be that mount, and `url`
    /// must be a page that mount may show (`UrlPolicy::navigation` with its ports, class and Lan host). A script
    /// that ran in a newer mount, or a document the policy would refuse, answers nothing usable.
    ///
    /// # Errors
    /// The reason, for the agent.
    pub fn reading_allowed(
        &self,
        session_id: &str,
        accessor: &str,
        url: &str,
    ) -> Result<(), String> {
        let state = self.inner.lock();
        let entry = state
            .entries
            .get(session_id)
            .ok_or_else(|| "this session has no preview open".to_owned())?;
        let mount = entry
            .ready_mount()
            .filter(|m| m.page == accessor)
            .ok_or_else(|| {
                "the page was replaced while it was read: call browser_read again".to_owned()
            })?;
        if url.is_empty() {
            return Err(
                "the page gave no address with its answer: call browser_read again".to_owned(),
            );
        }
        if url == "about:blank" {
            return Ok(());
        }
        let parsed = url::Url::parse(url)
            .map_err(|_| "the page reported an unreadable address".to_owned())?;
        if self.inner.policy.is_canary(&parsed) {
            return Err("the page is still loading: call browser_read again".to_owned());
        }
        self.inner
            .policy
            .navigation(&parsed, &entry.allowed, mount.class, entry.mount_lan_host())
            .map_err(|why| {
                format!("the page that answered is not one this browser reads here ({why})")
            })
    }

    /// Where a capture may fetch `url` when it is on the local-network page the USER opened in `session_id` by NAME:
    /// `None` when `url`'s host is not that page's own host name (the ordinary public-only fetch applies; a literal address already goes that way); else that name's addresses from the lookup the
    /// navigation uses, accepted only when every answer is a private-network, carrier-grade NAT or public address
    /// (never this computer, link-local or reserved: the same rule as the network guard for the user's Lan page).
    /// The fetcher connects to exactly these addresses (no second lookup).
    pub async fn lan_media_addresses(
        &self,
        session_id: &str,
        url: &url::Url,
    ) -> Option<Result<Vec<std::net::SocketAddr>, String>> {
        use vw_netguard::ip::{IpClass, classify};
        let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
        if host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok()
        {
            return None;
        }
        let own = {
            let state = self.inner.lock();
            let entry = state.entries.get(session_id)?;
            entry
                .mount_lan_host()?
                .trim_end_matches('.')
                .to_ascii_lowercase()
        };
        if own != host {
            return None;
        }
        let port = url.port_or_known_default()?;
        let resolver = self.inner.names.resolver();
        let name = host.clone();
        let lookup = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            tokio::task::spawn_blocking(move || resolver(&name)),
        )
        .await;
        let answers = match lookup {
            Ok(Ok(Ok(answers))) if !answers.is_empty() => answers,
            Ok(Ok(Ok(_))) => return Some(Err(format!("{host} has no address"))),
            Ok(Ok(Err(error))) => {
                return Some(Err(format!("{host} could not be resolved ({error})")));
            }
            Ok(Err(_)) | Err(_) => return Some(Err(format!("{host} did not resolve in time"))),
        };
        if let Some(bad) = answers.iter().find(|ip| {
            !matches!(
                classify(**ip),
                IpClass::Public | IpClass::Private | IpClass::SharedCgnat
            )
        }) {
            return Some(Err(format!(
                "{host} resolves to {bad}, {}",
                classify(*bad).describe()
            )));
        }
        Some(Ok(answers
            .into_iter()
            .map(|ip| std::net::SocketAddr::new(ip, port))
            .collect()))
    }

    /// Whether a capture of the session's current page may fetch `url` (media): a public host for every
    /// page (the fetcher checks the DNS answers again when it connects and pins them), anything else only as the
    /// mount's own navigation policy allows (the session's own ports for a page of its local server, the Lan page's
    /// own host). Returns where the URL goes.
    ///
    /// # Errors
    /// The reason.
    pub fn fetch_allowed(
        &self,
        session_id: &str,
        url: &url::Url,
    ) -> Result<crate::policy::Destination, String> {
        let destination = self.inner.policy.classify(url).map_err(|e| e.to_string())?;
        if destination == crate::policy::Destination::Public {
            return Ok(destination);
        }
        let state = self.inner.lock();
        let entry = state
            .entries
            .get(session_id)
            .ok_or_else(|| "this session has no preview open".to_owned())?;
        let mount = entry
            .ready_mount()
            .ok_or_else(|| "this session's page is not shown".to_owned())?;
        self.inner
            .policy
            .navigation(url, &entry.allowed, mount.class, entry.mount_lan_host())
            .map_err(|e| e.to_string())?;
        Ok(destination)
    }

    /// The host of the local-network page the USER opened in the mount `label` (`None`: not a Lan mount, or gone);
    /// the network guard lets that mount's requests reach this one host.
    #[must_use]
    pub fn lan_host_of(&self, label: &str) -> Option<String> {
        let state = self.inner.lock();
        let session = state.session_of(label)?;
        let entry = state.entries.get(session)?;
        entry
            .mount
            .as_ref()
            .filter(|m| m.label == label)
            .and(entry.mount_lan_host())
            .map(|h| {
                h.trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_ascii_lowercase()
            })
    }

    /// A mark of the session's console so far (every entry ever collected, dropped ones included), after draining
    /// the page; [`Self::console_since`] answers what came after it.
    ///
    /// # Errors
    /// [`PreviewError`] when the drain fails.
    pub async fn console_mark(&self, session_id: &str) -> Result<u64, PreviewError> {
        self.console(session_id, Some(0)).await?;
        let state = self.inner.lock();
        Ok(state.entries.get(session_id).map_or(0, |e| e.console_seen))
    }

    /// Console entries collected after `mark` (newest last, at most [`MAX_RETURNED`]).
    ///
    /// # Errors
    /// [`PreviewError`] when the drain fails.
    pub async fn console_since(
        &self,
        session_id: &str,
        mark: u64,
    ) -> Result<Vec<ConsoleEntry>, PreviewError> {
        self.console(session_id, Some(0)).await?;
        let state = self.inner.lock();
        let Some(entry) = state.entries.get(session_id) else {
            return Ok(Vec::new());
        };
        let new = usize::try_from(entry.console_seen.saturating_sub(mark))
            .unwrap_or(usize::MAX)
            .min(entry.console.len())
            .min(MAX_RETURNED);
        let skip = entry.console.len() - new;
        Ok(entry.console.iter().skip(skip).cloned().collect())
    }
}
