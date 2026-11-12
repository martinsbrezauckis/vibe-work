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
    /// The live mount belongs to this session; retained for older callers.
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
        let _ = self.protect_document(session_id, false);
    }

    /// Atomically excludes agent scripts before a vault password enters the document.
    ///
    /// # Errors
    /// Page code already ran in this document and could observe a later fill.
    pub fn admit_vault_fill(&self, session_id: &str) -> Result<(), String> {
        self.protect_document(session_id, true)
    }

    fn protect_document(&self, session_id: &str, exclude_scripts: bool) -> Result<(), String> {
        let mut state = self.inner.lock();
        if exclude_scripts
            && state
                .entries
                .get(session_id)
                .is_some_and(|entry| entry.scripted_at == Some(*entry.loads.borrow()))
        {
            return Err("Vault fill is blocked because page JavaScript ran in this document and may retain a password listener; reload or navigate to a new document before filling.".into());
        }
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
        Ok(())
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

    /// Compatibility view of a live mount's owner. Address classes do not restrict browser requests.
    /// Stale labels remain refused; current guard callers use the live mount identity directly.
    #[must_use]
    pub fn loopback_gate(&self, label: &str) -> LoopbackGate {
        let state = self.inner.lock();
        let Some(session) = state.session_of(label) else {
            return LoopbackGate::Refuse("this page is no longer shown".to_owned());
        };
        if state
            .entries
            .get(session)
            .and_then(|entry| entry.mount.as_ref())
            .is_some_and(|mount| mount.label == label)
        {
            LoopbackGate::NeedOwner(state.owner_of(session))
        } else {
            LoopbackGate::Refuse("this page is no longer shown".to_owned())
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
        self.frame_reading_allowed(session_id, accessor, url)?;
        let state = self.inner.lock();
        let engine = state
            .entries
            .get(session_id)
            .filter(|entry| {
                entry
                    .ready_mount()
                    .is_some_and(|mount| mount.page == accessor)
            })
            .and_then(|entry| entry.url.as_ref());
        let reported =
            url::Url::parse(url).map_err(|_| "the page reported an unreadable address")?;
        if !matches!(reported.scheme(), "http" | "https")
            || engine.is_none_or(|engine| engine.origin() != reported.origin())
        {
            return Err(
                "the page address differs from the browser engine: call browser_read again".into(),
            );
        }
        Ok(())
    }

    /// Checks an engine-identified frame against this mount's network policy. Its caller must verify the frame's
    /// native origin and document identity; cross-origin children are legitimate and are not the top document.
    ///
    /// # Errors
    /// A stale mount or address forbidden by its network policy.
    pub fn frame_reading_allowed(
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
        // Documents the page made itself: read under the page's own rule (the frame reader checks the parent first).
        if matches!(url, "about:blank" | "about:srcdoc") || url.starts_with("data:") {
            return Ok(());
        }
        let mut parsed = url::Url::parse(url)
            .map_err(|_| "the page reported an unreadable address".to_owned())?;
        // A `blob:` document belongs to the origin that made it: that origin's rule applies.
        if parsed.scheme() == "blob" {
            parsed = url::Url::parse(&format!("{}/", parsed.origin().ascii_serialization()))
                .map_err(|_| "the page reported an unreadable address".to_owned())?;
        }
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

    /// Resolves and pins media addresses only when the main document committed with wholly private/CGNAT
    /// connection evidence. Later DNS and child frames cannot grant access. Public or unproven loads use
    /// the fetcher's public-only resolver. Loopback, link-local and reserved answers are always refused.
    /// `None` retains the public-only rule; `Some` contains every checked destination address, with no second lookup.
    pub async fn lan_media_addresses(
        &self,
        session_id: &str,
        url: &url::Url,
    ) -> Option<Result<Vec<std::net::SocketAddr>, String>> {
        use vw_netguard::ip::{IpClass, classify};
        let document = {
            let state = self.inner.lock();
            let mount = state.entries.get(session_id)?.ready_mount()?;
            (
                mount.label.clone(),
                mount.navigation.load_addresses.private_generation()?,
            )
        };
        let host = url.host_str()?;
        let port = url.port_or_known_default()?;
        let answers = match resolve_media_host(self.inner.names.resolver(), host).await {
            Ok(answers) => answers,
            Err(error) => return Some(Err(error)),
        };
        {
            let state = self.inner.lock();
            let mount = state.entries.get(session_id)?.ready_mount()?;
            if mount.label != document.0
                || mount.navigation.load_addresses.private_generation() != Some(document.1)
            {
                return None;
            }
        }
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

    /// Whether a capture may fetch `url`, with checked addresses for a private page's media.
    /// Public pages retain the public-only resolver and the existing session-port restrictions.
    ///
    /// # Errors
    /// The reason.
    pub async fn fetch_allowed(
        &self,
        session_id: &str,
        url: &url::Url,
    ) -> Result<
        (
            crate::policy::Destination,
            Option<Vec<std::net::SocketAddr>>,
        ),
        String,
    > {
        let destination = self
            .inner
            .policy
            .fetch_classify(url)
            .map_err(|e| e.to_string())?;
        if matches!(
            destination,
            crate::policy::Destination::Public | crate::policy::Destination::Lan
        ) && let Some(addresses) = self.lan_media_addresses(session_id, url).await
        {
            return addresses.map(|addresses| (destination, Some(addresses)));
        }
        if destination == crate::policy::Destination::Lan {
            return Err(crate::policy::UrlRefusal::Lan(
                url.host_str().unwrap_or_default().to_owned(),
            )
            .to_string());
        }
        if destination == crate::policy::Destination::Public {
            return Ok((destination, None));
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
            .fetch_navigation(url, &entry.allowed, mount.class, entry.mount_lan_host())
            .map_err(|e| e.to_string())?;
        Ok((destination, None))
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

// Resolve once; the caller classifies the current page and pins the checked media addresses.
async fn resolve_media_host(
    resolver: crate::names::NameResolver,
    host: &str,
) -> Result<Vec<std::net::IpAddr>, String> {
    let host = host
        .trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let answers = if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        vec![ip]
    } else {
        let name = host.clone();
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            tokio::task::spawn_blocking(move || resolver(&name)),
        )
        .await
        .map_err(|_| format!("{host} did not resolve in time"))?
        .map_err(|_| format!("{host} could not be resolved"))??
    };
    if answers.is_empty() {
        return Err(format!("{host} has no address"));
    }
    Ok(answers)
}
