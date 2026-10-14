//! The engine's callbacks into the registry ([`PreviewHooks`]): navigation decisions, page events, refused new
//! windows and download destinations. Called on the engine's UI thread: short critical sections, no blocking I/O
//! beyond creating the downloads folder.
//!
//! Depends on: `state`, `policy`, `downloads`, `engine`. Called by: the engine (through `PreviewService::hooks`).
//! Invariants:
//! - An unknown label (closed or never mounted) is refused everything.
//! - A navigation is allowed only by `UrlPolicy::navigation` with the entry's allowed ports, the mount's page
//!   class and (Lan mounts) its own host, and, for a host NAME, only when every DNS answer is public
//!   (`names`, failing closed; the lookup runs outside the registry lock). A Lan mount's own host is not looked up
//!   (the user opened it). Sub-requests asked through `request_allowed` / `request_check` (the Windows barrier) get
//!   the same name check; the engine resolves again on connect, so only the Linux guard pins the checked address.
//! - No callback waits on a lookup for more than `names::UI_WAIT`: a lookup still running
//!   lets a navigation go on only when the engine checks names again at request time (`names_checked_later`: the
//!   Linux proxy, the `WebView2` barrier, which holds the request with a deferral via `request_check`), else refuses
//!   it with "open it again in a moment" while the lookup fills the cache.
//! - Only the canary URL the engine announced for a mount's pending guard check is allowed (until the check ends) and
//!   never recorded as the page; any other canary URL is refused.
//! - A mount keeps its page class (web or the session's local server): a navigation inside it to the other class is
//!   refused (top level or in a frame; `WebKitGTK` asks the same policy for both) and opens only as a fresh mount.
//! - A page that ends up outside its allowed ports (an engine that does not ask about a redirect) is unloaded:
//!   `page_event` answers `false`.
//! - The engine writes downloads into the app's private staging folder only; a finished one is delivered into the
//!   session's `downloads/` folder by `downloads::deliver` on a helper thread (never through a link, never over an
//!   existing file, size-capped). Without a session folder they are cancelled.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use url::Url;

use crate::downloads::{self, DownloadState, DownloadView};
use crate::engine::{GuardCheck, GuardLayer, PageEvent, PreviewHooks, RequestCheck};
use crate::policy::{Destination, PageClass, UrlPolicy};
use crate::state::DownloadRecord;
use crate::state::State;

/// Two download starts of the same URL within this window are the same download (`WebKitGTK` can call a web
/// context's handler once per webview that ever registered on it).
const DUPLICATE_WINDOW: Duration = Duration::from_secs(2);

/// Shared registry + policy.
pub(crate) struct Inner {
    pub(crate) state: Mutex<State>,
    pub(crate) policy: UrlPolicy,
    /// Session ids whose preview changed (page loaded, title, refusal, download), for the UI's refresh.
    pub(crate) changes: tokio::sync::broadcast::Sender<String>,
    /// The private folder the engine writes downloads into.
    pub(crate) staging: PathBuf,
    /// Downloads above this size are dropped.
    pub(crate) max_download_bytes: u64,
    /// The network guard, for Vibe Work's own ports.
    pub(crate) guard: Mutex<Option<vw_netguard::NetGuard>>,
    /// Where host names point, for in-place navigations.
    pub(crate) names: crate::names::NameCheck,
    /// One URL-rule refresh at a time (each computes from the newest state and finishes before the
    /// next starts, so an older, wider list can never be applied after a newer one).
    pub(crate) rules_lock: tokio::sync::Mutex<()>,
}

impl Inner {
    /// Tells subscribers that `session_id`'s preview changed (none listening is fine).
    pub(crate) fn changed(&self, session_id: &str) {
        let owner = self.lock().owner_of(session_id);
        let _ = self.changes.send(owner);
    }

    /// Vibe Work's own ports (the guard's live list); `None` while no guard is known (callers fail closed).
    pub(crate) fn internal_ports(&self) -> Option<Vec<u16>> {
        self.guard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(vw_netguard::NetGuard::internal_ports)
    }

    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A host name a navigation or sub-request must be looked up for. `lan`: it is the host of the Lan page the user
/// opened, so a local-network answer is accepted for it ([`crate::names::NameVerdict::refusal_on`]).
struct NameCheck {
    host: String,
    lan: bool,
}

/// The host name a navigation must be looked up for: a `Host::Domain` other than `localhost` and the canary suffix
/// (`None`: nothing to look up). The Lan mount's own host IS looked up (to limit DNS rebinding):
/// only its local-network answer is accepted, never loopback, link-local or special ones.
fn name_to_check(url: &Url, lan_host: Option<&str>) -> Option<NameCheck> {
    let url::Host::Domain(name) = url.host()? else {
        return None;
    };
    let lower = name.to_ascii_lowercase();
    (lower != "localhost" && !lower.ends_with(vw_netguard::CANARY_SUFFIX)).then(|| NameCheck {
        lan: lan_host.is_some_and(|h| h.eq_ignore_ascii_case(&lower)),
        host: lower,
    })
}

/// What a navigation needs after the policy: nothing more, a refusal, or a name lookup.
enum Pre {
    /// Decided: allowed, or refused (`None`: an unknown label, nothing to record).
    Done(Result<(), Option<(String, String)>>),
    /// The policy allows it; `host` must point to public addresses. `later`: the engine checks names again at
    /// request time for this mount.
    Name {
        session: String,
        host: String,
        later: bool,
        lan: bool,
    },
}

/// The refusal text while a name's lookup is still running and nothing later checks it.
fn pending_text(host: &str) -> String {
    format!("checking where {host} points; open it again in a moment")
}

impl Hooks {
    /// The policy part of a navigation decision (under the registry lock, never waiting).
    fn pre(&self, label: &str, url: &Url) -> Pre {
        let mut state = self.0.lock();
        let Some(session) = state.session_of(label).map(str::to_owned) else {
            return Pre::Done(Err(None));
        };
        let pinned_app =
            state.tabs.contains_key(&session) && !state.page_owners.contains_key(&session);
        let entry = state.entry(&session);
        if entry.canary.as_ref() == Some(url) {
            return Pre::Done(Ok(()));
        }
        if pinned_app
            && matches!(
                self.0.policy.classify(url),
                Ok(Destination::Public | Destination::Lan)
            )
        {
            return Pre::Done(Err(Some((
                session,
                "the App preview opens only this session's local servers".to_owned(),
            ))));
        }
        let (class, lan_host) = (entry.mount_class(), entry.mount_lan_host());
        if let Err(reason) = self
            .0
            .policy
            .navigation(url, &entry.allowed, class, lan_host)
        {
            return Pre::Done(Err(Some((session, reason.to_string()))));
        }
        let later = entry
            .mount
            .as_ref()
            .is_some_and(|m| m.label == label && m.names_later);
        match name_to_check(url, lan_host) {
            Some(NameCheck { host, lan }) => Pre::Name {
                session,
                host,
                later,
                lan,
            },
            None => Pre::Done(Ok(())),
        }
    }

    /// The navigation decision without ever waiting more than [`crate::names::UI_WAIT`] (the engine's UI thread):
    /// a known verdict decides; a lookup still running lets the navigation go on when the engine checks names again
    /// at request time (`later`), else refuses it (fail closed; the lookup goes on and fills the cache).
    fn decide(&self, label: &str, url: &Url) -> Result<(), Option<(String, String)>> {
        let (session, host, later, lan) = match self.pre(label, url) {
            Pre::Done(result) => return result,
            Pre::Name {
                session,
                host,
                later,
                lan,
            } => (session, host, later, lan),
        };
        match self.0.names.quick(&host, crate::names::UI_WAIT) {
            Some(verdict) => match verdict.refusal_on(lan) {
                None => Ok(()),
                Some(why) => Err(Some((session, format!("{host} is not opened: {why}")))),
            },
            None if later => Ok(()),
            None => Err(Some((session, pending_text(&host)))),
        }
    }

    /// Records a refused navigation for the receipts and the UI.
    fn record(&self, session: &str, url: &Url, reason: &str) {
        let mut state = self.0.lock();
        if let Some(entry) = state.entries.get_mut(session) {
            entry.note_refused(url.as_str(), reason);
        }
        drop(state);
        tracing::info!(
            session = %session,
            scheme = url.scheme(),
            port = ?url.port_or_known_default(),
            %reason,
            "preview navigation refused"
        );
        self.0.changed(session);
    }

    /// The policy part of a sub-request decision: `Ok(None)` send it, `Ok(Some(check))` only if the name's verdict
    /// allows it ([`NameCheck`]), `Err(())` refuse it.
    fn request_pre(&self, label: &str, url: &Url) -> Result<Option<NameCheck>, ()> {
        // WebSockets are judged like their HTTP equivalent; `data:` / `blob:` never leave the page.
        let mut probe = url.clone();
        match url.scheme() {
            "data" | "blob" => return Ok(None),
            "ws" => {
                let _ = probe.set_scheme("http");
            }
            "wss" => {
                let _ = probe.set_scheme("https");
            }
            _ => {}
        }
        let port = match self.0.policy.classify(&probe) {
            Ok(Destination::Canary | Destination::Blank) => return Ok(None),
            // A host NAME goes through the same cached resolver as navigations (a public
            // page's `<img src="http://nas.lan/">` must not reach the home network), failing closed; the Lan page's
            // own host is looked up too and may only point to the local network. The engine resolves again when it connects, so a rebinding name can still
            // change its answer in between: only the Linux guard pins the address.
            Ok(Destination::Public) => {
                let lan_host = {
                    let state = self.0.lock();
                    state
                        .session_of(label)
                        .and_then(|s| state.entries.get(s))
                        .filter(|e| e.mount.as_ref().is_some_and(|m| m.label == label))
                        .and_then(|e| e.mount_lan_host().map(str::to_owned))
                };
                return Ok(name_to_check(&probe, lan_host.as_deref()));
            }
            Ok(Destination::Loopback { port }) => port,
            // A local-network address only from the Lan page the user opened on that host.
            Ok(Destination::Lan) => {
                let state = self.0.lock();
                let own = state.session_of(label).is_some_and(|session| {
                    state.entries.get(session).is_some_and(|e| {
                        e.mount.as_ref().is_some_and(|m| m.label == label)
                            && e.mount_lan_host().is_some()
                            && e.mount_lan_host() == probe.host_str()
                    })
                });
                return if own { Ok(None) } else { Err(()) };
            }
            Err(_) => return Err(()),
        };
        // Loopback (the same owner rule as the Linux guard, decided synchronously): only from a page of the session's
        // own server (Local mount), never Vibe Work's own ports (fail closed while that list is unknown), only ports
        // of this session or of another session's preview (the cross-session backend).
        let Some(internal) = self.0.internal_ports() else {
            return Err(());
        };
        if internal.contains(&port) {
            return Err(());
        }
        let state = self.0.lock();
        let Some(session) = state.session_of(label) else {
            return Err(());
        };
        let local = state
            .entries
            .get(session)
            .and_then(|e| e.mount.as_ref())
            .is_some_and(|m| m.label == label && m.class == PageClass::Local);
        if local && state.entries.values().any(|e| e.allowed.contains(&port)) {
            Ok(None)
        } else {
            Err(())
        }
    }
}

/// Records `layer` on the entry's current mount (once).
fn add_layer(entry: &mut crate::state::Entry, layer: GuardLayer) {
    if !entry.layers.contains(&layer) {
        entry.layers.push(layer);
    }
}

/// The hooks object handed to the engine.
pub(crate) struct Hooks(pub(crate) Arc<Inner>);

impl PreviewHooks for Hooks {
    fn allow_navigation(&self, label: &str, url: &Url) -> bool {
        match self.decide(label, url) {
            Ok(()) => {
                // A fresh navigation is the only evidence that the replaced page's URL may be
                // current again. Its late load callbacks otherwise remain stale after open ends.
                let mut state = self.0.lock();
                if let Some(session) = state.session_of(label).map(str::to_owned) {
                    let entry = state.entry(&session);
                    if entry.pending_open.as_ref() == Some(url) {
                        entry.pending_requested_admitted = Some(url.clone());
                    } else if entry.pending_requested_admitted.is_some()
                        && entry.pending_previous_url.as_ref() != Some(url)
                        && !entry.pending_admitted_urls.contains(url)
                    {
                        // A redirect may be the first load event the engine reports. Retain a few
                        // guarded candidates so another admission cannot overwrite the genuine one.
                        if entry.pending_admitted_urls.len() == 8 {
                            entry.pending_admitted_urls.remove(0);
                        }
                        entry.pending_admitted_urls.push(url.clone());
                    }
                    if entry.pending_open.is_none()
                        && entry.pending_previous_url.as_ref() == Some(url)
                    {
                        entry.pending_previous_url = None;
                    }
                }
                true
            }
            Err(None) => false,
            Err(Some((session, reason))) => {
                self.record(&session, url, &reason);
                false
            }
        }
    }

    fn navigation_allowed(&self, label: &str, url: &Url) -> bool {
        self.decide(label, url).is_ok()
    }

    fn request_allowed(&self, label: &str, url: &Url) -> bool {
        match self.request_pre(label, url) {
            Ok(None) => true,
            Ok(Some(NameCheck { host, lan })) => self
                .0
                .names
                .quick(&host, crate::names::UI_WAIT)
                .is_some_and(|v| v.refusal_on(lan).is_none()),
            Err(()) => false,
        }
    }

    fn request_check(&self, label: &str, url: &Url, document: bool) -> RequestCheck {
        let (host, record, lan) = if document {
            match self.pre(label, url) {
                Pre::Done(Ok(())) => return RequestCheck::Allow,
                Pre::Done(Err(_)) => return RequestCheck::Deny,
                Pre::Name {
                    session, host, lan, ..
                } => (host, Some(session), lan),
            }
        } else {
            match self.request_pre(label, url) {
                Ok(None) => return RequestCheck::Allow,
                Err(()) => return RequestCheck::Deny,
                Ok(Some(NameCheck { host, lan })) => (host, None, lan),
            }
        };
        if let Some(verdict) = self.0.names.quick(&host, Duration::ZERO) {
            return match verdict.refusal_on(lan) {
                None => RequestCheck::Allow,
                Some(why) => {
                    if let Some(session) = &record {
                        self.record(session, url, &format!("{host} is not opened: {why}"));
                    }
                    RequestCheck::Deny
                }
            };
        }
        let (names, inner, url) = (self.0.names.clone(), self.0.clone(), url.clone());
        RequestCheck::Later(Box::new(move |done| {
            let name = host.clone();
            names.when_done(
                &name,
                Box::new(move |verdict| {
                    if let (Some(why), Some(session)) = (verdict.refusal_on(lan), &record) {
                        Hooks(inner).record(session, &url, &format!("{host} is not opened: {why}"));
                    }
                    done(verdict.refusal_on(lan).is_none());
                }),
            );
        }))
    }

    fn names_checked_later(&self, label: &str) {
        let mut state = self.0.lock();
        let Some(session) = state.session_of(label).map(str::to_owned) else {
            return;
        };
        if let Some(mount) = state
            .entry(&session)
            .mount
            .as_mut()
            .filter(|m| m.label == label)
        {
            mount.names_later = true;
        }
    }

    fn network_guard(&self, label: &str, check: GuardCheck) {
        let mut state = self.0.lock();
        let Some(session) = state.session_of(label).map(str::to_owned) else {
            return;
        };
        let entry = state.entry(&session);
        match check {
            GuardCheck::Expect(url) => entry.canary = Some(url),
            GuardCheck::Done(active) => {
                entry.canary = None;
                entry.guarded = active;
                if active {
                    add_layer(entry, GuardLayer::EgressProxy);
                }
                // If a guard fails open, a guard the canaries did not prove may be bypassed, so the
                // navigation hook must not leave pending names to it, unless the mount's request barrier holds them
                // (the barrier checks names at request time whatever the proxy does).
                let barrier = entry.layers.contains(&GuardLayer::RequestBarrier);
                if !active
                    && !barrier
                    && let Some(mount) = entry.mount.as_mut().filter(|m| m.label == label)
                {
                    mount.names_later = false;
                }
                let layers: Vec<&str> = entry.layers.iter().map(|l| l.name()).collect();
                tracing::info!(session = %session, active, ?layers, "browser network guard checked");
            }
            GuardCheck::Layer(layer) => {
                add_layer(entry, layer);
                // The rules given at mount are in place (recorded only once confirmed).
                if layer == GuardLayer::ContentRules
                    && let Some(mount) = entry.mount.as_mut().filter(|m| m.label == label)
                    && let Some(offered) = mount.rules_offered.take()
                {
                    mount.rules = Some(offered);
                    mount.rules_pending = false;
                }
                tracing::info!(session = %session, layer = layer.name(), "browser protection layer installed");
            }
        }
    }

    fn page_event(&self, label: &str, event: PageEvent) -> bool {
        let mut state = self.0.lock();
        let Some(session) = state.session_of(label).map(str::to_owned) else {
            return false;
        };
        let policy = &self.0.policy;
        let entry = state.entry(&session);
        let mut keep = true;
        match event {
            // Canary loads of the pending guard check are never the page and never a reason to unload. The announced
            // canary stays allowed until the check ends (`Done`), not until a load event: WebKitGTK reports the
            // cancelled previous load as `Finished` with the NEW pending URI (observed in the app).
            PageEvent::Started(url) | PageEvent::Finished(url) | PageEvent::Probe(url)
                if entry.canary.as_ref() == Some(&url) || policy.is_canary(&url) =>
            {
                return true;
            }
            PageEvent::Started(url) | PageEvent::Finished(url) | PageEvent::Probe(url)
                if url.as_str() != "about:blank"
                    && !policy.allows_navigation(
                        &url,
                        &entry.allowed,
                        entry.mount_class(),
                        entry.mount_lan_host(),
                    ) =>
            {
                entry.note_refused(
                    url.as_str(),
                    "the page ended up outside what this browser opens",
                );
                entry.loading = false;
                entry.title = None;
                tracing::info!(session = %session, "preview page outside its ports: unloaded");
                keep = false;
            }
            PageEvent::Probe(_) => return true,
            PageEvent::Started(url) => {
                if let Some(requested) = &entry.pending_open {
                    if (entry.pending_previous_url.as_ref() == Some(&url) && &url != requested)
                        || (!entry.pending_started
                            && &url != requested
                            && !entry.pending_admitted_urls.contains(&url))
                    {
                        return true;
                    }
                    entry.pending_started = true;
                    entry.pending_requested_admitted = None;
                    entry.pending_admitted_urls.clear();
                    entry.title = None;
                } else if entry.pending_previous_url.as_ref() == Some(&url) {
                    return true;
                }
                entry.reset_console();
                entry.favicon = None;
                entry.loading = true;
                if url.as_str() != "about:blank" {
                    entry.url = Some(url);
                }
            }
            PageEvent::Finished(url) => {
                if entry.url.as_ref() != Some(&url)
                    || (entry.pending_open.is_some()
                        && (!entry.pending_started
                            || (entry.pending_previous_url.as_ref() == Some(&url)
                                && entry.pending_open.as_ref() != Some(&url))))
                {
                    return true;
                }
                entry.pending_open = None;
                entry.pending_requested_admitted = None;
                entry.pending_admitted_urls.clear();
                entry.pending_started = false;
                entry.loading = false;
                if url.as_str() != "about:blank" {
                    entry.url = Some(url);
                }
                entry.loads.send_modify(|n| *n += 1);
                // Another document ends a vault fill's screenshot block.
                crate::browse::unprotect(label);
            }
            PageEvent::TitleAt { url, text } => {
                if entry.url.as_ref() == Some(&url)
                    && (entry.pending_open.is_none() || entry.pending_started)
                {
                    entry.title = Some(text.chars().take(200).collect());
                }
            }
            PageEvent::Title(title) if entry.pending_open.is_none() => {
                entry.title = Some(title.chars().take(200).collect());
            }
            PageEvent::Title(_) => {}
        }
        drop(state);
        self.0.changed(&session);
        keep
    }

    fn new_window_refused(&self, label: &str, url: &Url) {
        let mut state = self.0.lock();
        if let Some(session) = state.session_of(label).map(str::to_owned) {
            state
                .entry(&session)
                .note_refused(url.as_str(), "pages cannot open new windows here");
            drop(state);
            self.0.changed(&session);
        }
    }

    fn download_destination(&self, label: &str, url: &Url, suggested: &Path) -> Option<PathBuf> {
        let mut state = self.0.lock();
        let session = state.session_of(label)?.to_owned();
        let entry = state.entry(&session);
        let dir = entry.downloads_dir.clone()?;
        if let Some(same) = entry.downloads.iter().rev().find(|d| {
            d.view.url == url.as_str()
                && d.view.state == DownloadState::Running
                && d.at.elapsed() < DUPLICATE_WINDOW
        }) {
            return Some(same.staging.clone());
        }
        let name = suggested
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty())
            .map_or_else(
                || {
                    url.path_segments()
                        .and_then(|mut s| s.next_back())
                        .unwrap_or("")
                        .to_owned()
                },
                str::to_owned,
            );
        let name = downloads::clean_name(&name);
        let staging = downloads::staging_file(&self.0.staging).ok()?;
        entry.push_download(DownloadRecord {
            view: DownloadView {
                file_name: name.clone(),
                path: String::new(),
                url: url.to_string(),
                state: DownloadState::Running,
            },
            at: std::time::Instant::now(),
            staging: staging.clone(),
            dir,
            name,
        });
        drop(state);
        self.0.changed(&session);
        Some(staging)
    }

    fn download_finished(&self, label: &str, url: &Url, _path: Option<&Path>, ok: bool) {
        let mut state = self.0.lock();
        let Some(session) = state.session_of(label).map(str::to_owned) else {
            return;
        };
        let entry = state.entry(&session);
        // The engine's own path is not trusted (macOS reports none); the record knows the staged file.
        let Some(record) = entry
            .downloads
            .iter()
            .rev()
            .find(|d| d.view.state == DownloadState::Running && d.view.url == url.as_str())
        else {
            return;
        };
        let (staging, dir, name) = (
            record.staging.clone(),
            record.dir.clone(),
            record.name.clone(),
        );
        drop(state);
        // WORKAROUND(wry 0.57 WebKitGTK, web_context.rs:314-385): one `failed` flag per web context survives earlier
        // failures, so a good download can be reported failed. A staged file with content counts as written.
        // Exit: wry tracks failure per download.
        let written = ok || std::fs::metadata(&staging).is_ok_and(|m| m.is_file() && m.len() > 0);
        let inner = self.0.clone();
        let max = self.0.max_download_bytes;
        // Delivery copies the file: off the UI thread.
        std::thread::spawn(move || {
            let outcome = if written {
                downloads::deliver(&staging, &dir, &name, max)
            } else {
                let _ = std::fs::remove_file(&staging);
                Err("the download failed".to_owned())
            };
            if let Err(reason) = &outcome {
                tracing::info!(%reason, "preview download not saved");
            }
            let mut state = inner.lock();
            if let Some(entry) = state.entries.get_mut(&session)
                && let Some(record) = entry.downloads.iter_mut().find(|d| d.staging == staging)
            {
                match outcome {
                    Ok(path) => {
                        record.view.path = path.display().to_string();
                        record.view.state = DownloadState::Done;
                    }
                    Err(_) => record.view.state = DownloadState::Failed,
                }
            }
            drop(state);
            inner.changed(&session);
        });
    }
}
