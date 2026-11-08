//! The engine's callbacks into the registry ([`PreviewHooks`]): navigation decisions, page events, refused new
//! windows and download destinations. Called on the engine's UI thread: short critical sections, no blocking I/O
//! beyond creating the downloads folder.
//!
//! Depends on: `state`, `policy`, `downloads`, `engine`. Called by: the engine (through `PreviewService::hooks`).
//! Navigation and requests retain live-mount, supported-scheme, credential, canary and pinned App preview checks.
//! The browser engine resolves destinations; backend capture has its own address policy.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

mod requests;

use url::Url;

use crate::downloads::{self, DownloadState, DownloadView};
use crate::engine::{GuardCheck, GuardLayer, PageEvent, PreviewHooks, RequestCheck};
use crate::policy::{Destination, UrlPolicy};
use crate::state::DownloadRecord;
use crate::state::{Seen, State};

/// Two download starts of the same URL within this window are the same download (`WebKitGTK` can call a web
/// context's handler once per webview that ever registered on it).
const DUPLICATE_WINDOW: Duration = Duration::from_secs(2);

/// Shared registry + policy.
pub(crate) type RefusalListener = Arc<dyn Fn(&str, &str) + Send + Sync>;

pub(crate) type PopupListener = Arc<dyn Fn(crate::MountId, Url) -> bool + Send + Sync>;

pub(crate) struct Inner {
    pub(crate) popup: Mutex<Option<PopupListener>>,
    pub(crate) shields: Mutex<crate::shields::Saved>,
    pub(crate) guard_refused: Mutex<Option<RefusalListener>>,
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
    pub(crate) fn guard_refused(&self, session: &str, reason: &str) {
        let listener = self
            .guard_refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(listener) = listener {
            listener(session, reason);
        }
    }

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

/// What a navigation needs after the policy: nothing more, a refusal, or a name lookup.
pub(crate) enum Pre {
    /// Decided: allowed, or refused (`None`: an unknown label, nothing to record).
    Done(Result<(), Option<(String, String)>>),
}

impl Hooks {
    /// Logs a blocked background request without presenting it as a refused agent action.
    fn record_request(&self, label: &str, url: &Url) {
        if crate::policy::embedded_document(url) {
            tracing::debug!(
                scheme = url.scheme(),
                "embedded document needs no refusal notice"
            );
            return;
        }
        let session = {
            let state = self.0.lock();
            match state.session_of(label) {
                Some(session)
                    if state
                        .entries
                        .get(session)
                        .is_some_and(|entry| entry.canary_favicon(url)) =>
                {
                    tracing::debug!(session = %session, "the canary page's favicon request refused (not reported)");
                    None
                }
                other => other.map(str::to_owned),
            }
        };
        if let Some(session) = session {
            tracing::info!(
                %session, scheme = url.scheme(), port = ?url.port_or_known_default(),
                "browser background request refused"
            );
        }
    }
    /// The policy part of a top-level navigation decision (under the registry lock, never waiting).
    pub(crate) fn pre(&self, label: &str, url: &Url) -> Pre {
        self.pre_in(label, url, false)
    }

    /// The policy part of a navigation decision of the page (`frame` false) or one of its frames, which may also load
    /// the page's own inline document ([`crate::policy::UrlPolicy::frame_navigation`]).
    fn pre_in(&self, label: &str, url: &Url, frame: bool) -> Pre {
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
        // Our own canary after its check ended: refused, and not recorded (the user never asked for it).
        if entry.ended_canaries.contains(url) {
            return Pre::Done(Err(None));
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
        // A child Blob document keeps the page's origin and follows the existing frame policy.
        let decision = if pinned_app
            && !crate::policy::embedded_document(url)
            && !(frame && url.scheme() == "blob")
        {
            self.0.policy.fetch_navigation(
                url,
                &entry.allowed,
                crate::policy::PageClass::Local,
                None,
            )
        } else if frame {
            self.0
                .policy
                .frame_navigation(url, &entry.allowed, class, lan_host)
        } else {
            self.0
                .policy
                .navigation(url, &entry.allowed, class, lan_host)
        };
        if let Err(reason) = decision {
            return Pre::Done(Err(Some((session, reason.to_string()))));
        }
        Pre::Done(Ok(()))
    }

    /// Checks live mount identity, supported schemes and pinned App preview ownership without resolving names.
    pub(crate) fn decide(&self, label: &str, url: &Url) -> Result<(), Option<(String, String)>> {
        self.decide_in(label, url, false)
    }

    /// [`Self::decide`] for the page (`frame` false) or one of its frames.
    fn decide_in(
        &self,
        label: &str,
        url: &Url,
        frame: bool,
    ) -> Result<(), Option<(String, String)>> {
        let Pre::Done(result) = self.pre_in(label, url, frame);
        result
    }

    /// Records a refused navigation for the receipts, the UI and the security bell, once per refusal: the other
    /// hook's report of the same navigation (`WebView2` reports one to both, see [`Seen`]) records nothing.
    fn record(&self, session: &str, url: &Url, reason: &str, seen: Seen) {
        if seen == Seen::Request {
            tracing::info!(
                %session, scheme = url.scheme(), port = ?url.port_or_known_default(), %reason,
                "browser background name lookup refused"
            );
            return;
        }
        if crate::policy::embedded_document(url) {
            tracing::debug!(
                scheme = url.scheme(),
                "embedded document needs no refusal notice"
            );
            return;
        }
        let mut state = self.0.lock();
        if let Some(entry) = state.entries.get_mut(session) {
            if entry.second_report(url, seen) {
                drop(state);
                tracing::debug!(session = %session, ?seen, "preview navigation refusal reported by its second hook");
                return;
            }
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
        self.0.guard_refused(session, reason);
        self.0.changed(session);
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
    fn content_filter(&self) -> Option<String> {
        Some(
            self.0
                .shields
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .rules
                .clone(),
        )
    }

    fn allow_frame_navigation(&self, label: &str, url: &Url) -> bool {
        match self.decide_in(label, url, true) {
            Ok(()) => true,
            Err(None) => false,
            Err(Some((session, reason))) => {
                self.record(&session, url, &reason, Seen::Navigation);
                false
            }
        }
    }

    fn allow_navigation(&self, label: &str, url: &Url) -> bool {
        self.begin_navigation(label);
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
                self.record(&session, url, &reason, Seen::Navigation);
                false
            }
        }
    }

    fn navigation_allowed(&self, label: &str, url: &Url) -> bool {
        self.decide(label, url).is_ok()
    }

    fn request_allowed(&self, label: &str, url: &Url) -> bool {
        let allowed = self.request_pre(label, url).is_ok();
        if !allowed {
            self.record_request(label, url);
        }
        allowed
    }

    fn request_check(&self, label: &str, url: &Url, document: bool) -> RequestCheck {
        let allowed = if document {
            let Pre::Done(result) = self.pre(label, url);
            match result {
                Ok(()) => true,
                Err(reason) => {
                    if let Some((session, reason)) = reason {
                        self.record(&session, url, &reason, Seen::Document);
                    }
                    false
                }
            }
        } else {
            self.request_allowed(label, url)
        };
        if allowed {
            RequestCheck::Allow
        } else {
            RequestCheck::Deny
        }
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
            GuardCheck::Expect(url) => {
                entry.end_canary();
                entry.canary = Some(url);
            }
            GuardCheck::Done(active) => {
                entry.end_canary();
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
        let pinned_app =
            state.tabs.contains_key(&session) && !state.page_owners.contains_key(&session);
        let entry = state.entry(&session);
        let mut keep = true;
        match event {
            // Canary loads are never the page and never a reason to unload, whenever the engine reports them: the
            // announced canary stays allowed until the check ends (`Done`), not until a load event (WebKitGTK reports
            // the cancelled previous load as `Finished` with the NEW pending URI, observed in the app), and a report
            // that arrives after `Done` is for a canary the engine announced (`ended_canaries`), not a page.
            PageEvent::Started(url) | PageEvent::Finished(url) | PageEvent::Probe(url)
                if entry.announced_canary(&url) || policy.is_canary(&url) =>
            {
                return true;
            }
            PageEvent::Started(url) | PageEvent::Finished(url) | PageEvent::Probe(url)
                if url.as_str() != "about:blank"
                    && !requests::page_allowed(policy, entry, pinned_app, &url) =>
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
            // The old document cannot finish a held load; the out-of-policy unload above still applies.
            _ if entry.navigation_held() => return true,
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
        if !keep {
            self.0.guard_refused(
                &session,
                "the page ended up outside what this browser opens",
            );
        }
        self.0.changed(&session);
        keep
    }

    fn new_window_requested(&self, label: &str, url: &Url) {
        self.route_popup(label, url);
    }
    fn act_refused(&self, label: &str, kind: crate::ActRefusal) {
        self.note_act_refusal(label, kind);
    }

    fn new_window_refused(&self, label: &str, url: &Url) {
        if crate::policy::embedded_document(url) {
            tracing::debug!(
                scheme = url.scheme(),
                "embedded document needs no refusal notice"
            );
            return;
        }
        let mut state = self.0.lock();
        if let Some(session) = state.session_of(label).map(str::to_owned) {
            state
                .entry(&session)
                .note_refused(url.as_str(), "pages cannot open new windows here");
            drop(state);
            self.0
                .guard_refused(&session, "pages cannot open new windows here");
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
