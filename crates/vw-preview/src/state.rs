//! The preview registry's state: one entry per Browser page (its URL, title, mount, width, allowed ports, console
//! ring, downloads) plus what the window shows. Pure data with small transitions; the async service drives the
//! engine around it (`service`), the engine's callbacks update it (`hooks`).
//!
//! Depends on: `ids`, `layout`, `console`, `downloads`, url, tokio (`watch`). Called by: `service`, `hooks`.
//! Invariants:
//! - Page keys map to one owning session; a mount label maps to exactly one page (`by_label`), and a closed mount's label
//!   is forgotten at once, so a late callback of an old webview finds nothing and is refused.
//! - The live cap counts mounted entries; the shown session's entry is never the one suspended.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use url::Url;

use crate::console::{ConsoleEntry, HOST_RING};
use crate::downloads::{DownloadView, MAX_LISTED};
use crate::layout::{DeviceWidth, Rect};

/// Refused navigations remembered per preview.
pub const MAX_REFUSED: usize = 5;

/// How long a refusal one hook reported waits for the other hook's report of the same navigation (`WebView2` reports
/// one refused navigation twice, about 2 ms apart, see [`Seen`]). Longer than that pair, short enough that a later
/// refusal of the same address through the other hook is its own refusal.
pub(crate) const REFUSAL_PAIR_WINDOW: Duration = Duration::from_secs(1);

/// Which engine hook reported a refused navigation. `WebView2` reports one refused main-frame navigation (and a
/// redirect) to both: the navigation callback (wry's `NavigationStarting`, main frame only) and the document-request
/// barrier (`WebResourceRequested`, main frame and frames, because `WebView2` still sends a cancelled navigation's
/// GET). A frame's navigation reaches only the barrier, a non-web scheme (`tauri://`) only the callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    /// The navigation callback (`PreviewHooks::allow_navigation`).
    Navigation,
    /// The document-request barrier (`PreviewHooks::request_check` with `document`).
    Document,
    /// A sub-request's name check (`request_check` without `document`): never a navigation, never paired.
    Request,
}

/// The last refusal recorded for a page, waiting for its other half (see [`Entry::second_report`]).
#[derive(Clone, Debug)]
pub(crate) struct RefusalMark {
    url: String,
    seen: Seen,
    at: Instant,
}

/// Canaries of ended guard checks remembered per preview (two per check; a few remounts' worth).
pub(crate) const ENDED_CANARIES: usize = 8;

/// Whether an external browser may remount as layout updates arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowserMountState {
    /// A request or panel reopening may mount the browser.
    Ready,
    /// The last request found every browser slot occupied.
    Waiting,
    /// A completed turn released the browser until request or panel reopening.
    Released,
}

/// Whether a session's preview runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PreviewState {
    /// Mounted and shown in the window.
    Live,
    /// Mounted, hidden (another session or tab is shown).
    Hidden,
    /// Closed to respect the live cap; its URL is kept and it is mounted again when shown or used.
    Suspended,
    /// The local server formerly opened in this preview is no longer listening for this session.
    Stopped,
}

/// Current availability of the App preview's local server. An absent value means no stopped server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PreviewServerState {
    /// The server that supplied the open page has stopped or changed owner.
    Stopped,
}

/// A session's preview as the UI, the Debug API and the agent see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PreviewView {
    /// The session.
    pub session_id: String,
    /// The current URL (`None` before anything was opened).
    pub url: Option<String>,
    /// The page title (untrusted page text).
    pub title: Option<String>,
    /// Running state.
    pub state: PreviewState,
    /// Explicit stopped-server marker for the UI, Debug API and agent tools.
    pub server: Option<PreviewServerState>,
    /// True while a load is in progress.
    pub loading: bool,
    /// The latest external-browser request is waiting for a free engine.
    pub waiting_for_browser: bool,
    /// The device width.
    pub width: DeviceWidth,
    /// Recent downloads, newest last.
    pub downloads: Vec<DownloadView>,
    /// Recently refused navigations and new-window requests (untrusted URLs, newest last).
    pub refused: Vec<String>,
    /// Console errors and warnings collected so far (host side).
    pub console_count: u32,
}

/// One download: what is shown, and where the engine writes it until it is delivered.
pub(crate) struct DownloadRecord {
    pub(crate) view: DownloadView,
    pub(crate) at: std::time::Instant,
    /// The engine's file in the private staging folder.
    pub(crate) staging: PathBuf,
    /// The session's `downloads/` folder it goes to.
    pub(crate) dir: PathBuf,
    /// The cleaned file name it should get there.
    pub(crate) name: String,
}

/// Only a completed setup permits use; an unready child is retained solely for close retries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MountState {
    Unready,
    Ready,
}

/// The current mount of an entry.
#[derive(Clone, Debug)]
pub(crate) struct Mount {
    pub(crate) navigation: crate::navigation::Navigation,
    pub(crate) label: String,
    pub(crate) state: MountState,
    /// The page environment's accessor of this mount (`page_init::page_accessor`).
    pub(crate) page: String,
    /// Fixed for the mount's life: a navigation to the other class gets a fresh mount.
    pub(crate) class: crate::policy::PageClass,
    /// The engine checks host names again at request time for this mount (`PreviewHooks::names_checked_later`).
    pub(crate) names_later: bool,
    /// The URL rules the engine confirmed for this mount (its content-rules layer at mount, or an update
    /// that succeeded; `None`: none confirmed yet).
    pub(crate) rules: Option<vw_netguard::rules::RulePage>,
    /// The rules are not confirmed by the engine (given at mount and not yet reported applied, or an update
    /// failed): every later refresh sends them again.
    pub(crate) rules_pending: bool,
    /// The rules given at mount, recorded as applied once the engine reports its content-rules layer.
    pub(crate) rules_offered: Option<vw_netguard::rules::RulePage>,
}

/// One session's preview.
pub(crate) struct Entry {
    pub(crate) agent_navigation: usize,
    pub(crate) session_id: String,
    pub(crate) url: Option<Url>,
    pub(crate) title: Option<String>,
    /// Document-load epoch and normalized PNG icon; `None` pixels means an observed icon was inaccessible.
    pub(crate) favicon: Option<(u64, Option<String>)>,
    pub(crate) loading: bool,
    /// Set only by a successful ownership/listener re-check of the current local URL.
    pub(crate) server_status: Option<PreviewServerState>,
    /// The URL requested by `open` until that navigation finishes; older page events cannot complete its wait.
    pub(crate) pending_open: Option<Url>,
    /// The page replaced by the last open; its late events remain stale until a fresh navigation.
    pub(crate) pending_previous_url: Option<Url>,
    /// The requested URL passed the native navigation guard for this pending open.
    pub(crate) pending_requested_admitted: Option<Url>,
    /// Guarded redirect targets seen after that request, bounded so interleaved admissions cannot
    /// overwrite each other or accumulate without limit before the first load event.
    pub(crate) pending_admitted_urls: Vec<Url>,
    /// A load of `pending_open` has started in this mount.
    pub(crate) pending_started: bool,
    pub(crate) browser_mount: BrowserMountState,
    pub(crate) width: DeviceWidth,
    pub(crate) mount: Option<Mount>,
    /// Ports the preview may navigate to (attributed + leased when last opened or listed).
    pub(crate) allowed: Vec<u16>,
    /// Ordering for the live cap: higher = shown more recently.
    pub(crate) last_used: u64,
    pub(crate) console: VecDeque<ConsoleEntry>,
    pub(crate) console_dropped: u64,
    /// Advances on document changes so a drain from a previous page cannot enter this ring.
    pub(crate) console_epoch: u64,
    /// Lifetime count for browser action marks, which must survive a document change.
    pub(crate) console_seen: u64,
    pub(crate) downloads: Vec<DownloadRecord>,
    pub(crate) downloads_dir: Option<PathBuf>,
    pub(crate) refused: VecDeque<String>,
    /// This page's refused URLs with their reasons.
    pub(crate) refused_why: VecDeque<(String, String)>,
    /// Lifetime refusal count; action receipts must also see repeated refusals of the same URL.
    pub(crate) refusals_seen: u64,
    /// The last recorded refusal, until the other hook reports the same navigation (one record per refusal).
    pub(crate) refusal_mark: Option<RefusalMark>,
    /// The network guard is proven active for the current mount: public hosts may load.
    pub(crate) guarded: bool,
    /// The protection layers the engine reported for the current mount, besides the navigation policy.
    pub(crate) layers: Vec<crate::engine::GuardLayer>,
    /// The loopback canary the engine is loading right now (allowed once, never recorded as the page).
    pub(crate) canary: Option<Url>,
    /// Canaries the engine announced for checks that have ended, newest last, at most [`ENDED_CANARIES`]: the
    /// engine can report a canary's load after its check ended (`WebKitGTK`), or ask about it once more (`WebKit`);
    /// such a report is never the page and never shown as a refused page, and a navigation to it stays refused.
    pub(crate) ended_canaries: VecDeque<Url>,
    /// The local-network host the USER opened: set, the page is a [`PageClass::Lan`] page; its
    /// mount reaches only this host, and the agent neither reads nor acts on it. Kept across a suspension.
    ///
    /// [`PageClass::Lan`]: crate::policy::PageClass::Lan
    pub(crate) lan_host: Option<String>,
    /// Counts finished loads; `open` waits for it to move.
    pub(crate) loads: watch::Sender<u64>,
    /// screenshots are refused while the document whose load count this is stays shown (a vault password
    /// was filled into it; ShellX `shellx_browser_protected_values.rs`).
    pub(crate) shots_blocked_at: Option<u64>,
}

impl Entry {
    pub(crate) fn new(session_id: &str) -> Self {
        Self {
            agent_navigation: 0,
            session_id: session_id.to_owned(),
            url: None,
            title: None,
            favicon: None,
            loading: false,
            server_status: None,
            pending_open: None,
            pending_previous_url: None,
            pending_requested_admitted: None,
            pending_admitted_urls: Vec::new(),
            pending_started: false,
            browser_mount: BrowserMountState::Ready,
            width: DeviceWidth::Fit,
            mount: None,
            allowed: Vec::new(),
            last_used: 0,
            console: VecDeque::new(),
            console_dropped: 0,
            console_epoch: 0,
            console_seen: 0,
            downloads: Vec::new(),
            downloads_dir: None,
            refused: VecDeque::new(),
            refused_why: VecDeque::new(),
            refusals_seen: 0,
            refusal_mark: None,
            guarded: false,
            layers: Vec::new(),
            canary: None,
            ended_canaries: VecDeque::new(),
            lan_host: None,
            loads: watch::Sender::new(0),
            shots_blocked_at: None,
        }
    }

    /// A mount that completed every required setup step and may be used by service operations.
    pub(crate) fn ready_mount(&self) -> Option<&Mount> {
        self.mount
            .as_ref()
            .filter(|mount| mount.state == MountState::Ready)
    }

    /// The page class of the current mount (`Web` when not mounted: nothing local is reachable then).
    pub(crate) fn mount_class(&self) -> crate::policy::PageClass {
        self.mount
            .as_ref()
            .map_or(crate::policy::PageClass::Web, |m| m.class)
    }

    /// The Lan host of the current mount (only for a Lan mount).
    pub(crate) fn mount_lan_host(&self) -> Option<&str> {
        (self.mount_class() == crate::policy::PageClass::Lan)
            .then_some(self.lan_host.as_deref())
            .flatten()
    }

    /// Ends the pending canary (a new one is announced, or the check ended): remembered as ended, bounded.
    pub(crate) fn end_canary(&mut self) {
        if let Some(url) = self.canary.take() {
            self.ended_canaries.retain(|u| u != &url);
            self.ended_canaries.push_back(url);
            while self.ended_canaries.len() > ENDED_CANARIES {
                self.ended_canaries.pop_front();
            }
        }
    }

    /// True for a canary the engine announced for this preview: the pending one or one of an ended check.
    pub(crate) fn announced_canary(&self, url: &Url) -> bool {
        self.canary.as_ref() == Some(url) || self.ended_canaries.contains(url)
    }

    /// True for the engine's own `/favicon.ico` request of a canary page this preview showed (the same origin as an
    /// announced canary: for the loopback canary, the guard's proxy port). `WebView2` sends it just after the check.
    pub(crate) fn canary_favicon(&self, url: &Url) -> bool {
        url.path() == "/favicon.ico"
            && self
                .canary
                .iter()
                .chain(self.ended_canaries.iter())
                .any(|canary| canary.origin() == url.origin())
    }

    /// True when this refusal of `url` by hook `seen` is the other hook's report of the refusal recorded just before
    /// (the same address without its fragment, the other hook, within [`REFUSAL_PAIR_WINDOW`]): the caller then
    /// records nothing. Otherwise this refusal becomes the mark and the caller records it. A mark is used once, so
    /// three reports are two refusals, and two reports by the same hook are two refusals.
    pub(crate) fn second_report(&mut self, url: &Url, seen: Seen) -> bool {
        if seen == Seen::Request {
            return false;
        }
        let mut key = url.clone();
        key.set_fragment(None);
        let key: String = key.as_str().chars().take(200).collect();
        if let Some(mark) = self.refusal_mark.take()
            && mark.url == key
            && mark.seen != seen
            && mark.at.elapsed() < REFUSAL_PAIR_WINDOW
        {
            return true;
        }
        self.refusal_mark = Some(RefusalMark {
            url: key,
            seen,
            at: Instant::now(),
        });
        false
    }

    pub(crate) fn note_refused(&mut self, url: &str, reason: &str) {
        if Url::parse(url).is_ok_and(|url| crate::policy::embedded_document(&url)) {
            return;
        }
        let url: String = url.chars().take(200).collect();
        self.refused.push_back(url.clone());
        self.refusals_seen = self.refusals_seen.saturating_add(1);
        while self.refused.len() > MAX_REFUSED {
            self.refused.pop_front();
        }
        self.refused_why
            .push_back((url, reason.chars().take(300).collect()));
        while self.refused_why.len() > MAX_REFUSED {
            self.refused_why.pop_front();
        }
    }

    pub(crate) fn reset_console(&mut self) {
        self.console.clear();
        self.refused_why.clear();
        self.console_dropped = 0;
        self.console_epoch = self.console_epoch.wrapping_add(1);
    }

    pub(crate) fn push_console(&mut self, entries: Vec<ConsoleEntry>) {
        for entry in entries {
            self.console_seen = self.console_seen.saturating_add(1);
            self.console.push_back(entry);
            if self.console.len() > HOST_RING {
                self.console.pop_front();
                self.console_dropped += 1;
            }
        }
    }

    pub(crate) fn push_download(&mut self, record: DownloadRecord) {
        self.downloads.push(record);
        if self.downloads.len() > MAX_LISTED {
            self.downloads.remove(0);
        }
    }

    pub(crate) fn view(&self, shown: bool) -> PreviewView {
        let state = match (self.ready_mount(), shown) {
            _ if self.server_status.is_some() => PreviewState::Stopped,
            (Some(_), true) => PreviewState::Live,
            (Some(_), false) => PreviewState::Hidden,
            (None, _) => PreviewState::Suspended,
        };
        PreviewView {
            session_id: self.session_id.clone(),
            url: self.url.as_ref().map(ToString::to_string),
            title: self.title.clone(),
            state,
            server: self.server_status,
            loading: self.loading && self.server_status.is_none(),
            waiting_for_browser: self.browser_mount == BrowserMountState::Waiting,
            width: self.width,
            downloads: self.downloads.iter().map(|d| d.view.clone()).collect(),
            refused: self.refused.iter().cloned().collect(),
            console_count: u32::try_from(self.console.len()).unwrap_or(u32::MAX),
        }
    }
}

/// What the window shows: a session's preview at a rectangle.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shown {
    pub(crate) session_id: String,
    pub(crate) area: Rect,
}

/// The whole registry.
#[derive(Default)]
pub(crate) struct State {
    pub(crate) entries: HashMap<String, Entry>,
    /// The app's theme is light: the page's own menu falls back to it when the page shows no light or dark.
    pub(crate) light_app: bool,
    /// Opaque web-page key -> owning Vibe session. The pinned app page uses its session id directly.
    pub(crate) page_owners: HashMap<String, String>,
    /// Pages admitted for close; ownership cannot change before native teardown finishes.
    pub(crate) closing_pages: std::collections::HashSet<String>,
    pub(crate) tabs: HashMap<String, SessionTabs>,
    pub(crate) by_label: HashMap<String, String>,
    pub(crate) shown: Option<Shown>,
    /// Selected pages whose native child is parked under HTML Browser chrome, keyed by page and mount label.
    pub(crate) browser_overlays: HashMap<String, String>,
    /// Last prepared page identity: mount label, document load and child bounds.
    pub(crate) browser_overlay_prepared: HashMap<String, (String, u64, Rect)>,
    /// The last preview area the window reported: hidden previews are parked at the placement they would
    /// get there.
    pub(crate) last_area: Option<Rect>,
    pub(crate) clock: u64,
    pub(crate) generation: u64,
    /// Ports any session's server listens on now (from the core's re-check): a Local page may reach
    /// them (as the egress guard allows), so the URL rules include them.
    pub(crate) server_ports: Vec<u16>,
    /// Storage keys a page was mounted with in this run (that owner has website data of its own).
    pub(crate) used_keys: std::collections::HashSet<crate::ids::StorageKey>,
}

/// Ordered pages of one session; the app page is implicit and pinned first.
#[derive(Default)]
pub(crate) struct SessionTabs {
    pub(crate) web: Vec<String>,
    pub(crate) owners: HashMap<String, crate::tabs::PageTabOwner>,
    pub(crate) selected: Option<String>,
    pub(crate) agent: Option<String>,
    pub(crate) snapshots: HashMap<String, String>,
}

impl State {
    pub(crate) fn owner_of(&self, page_key: &str) -> String {
        self.page_owners
            .get(page_key)
            .cloned()
            .unwrap_or_else(|| page_key.to_owned())
    }
    pub(crate) fn entry(&mut self, session_id: &str) -> &mut Entry {
        self.entries
            .entry(session_id.to_owned())
            .or_insert_with(|| Entry::new(session_id))
    }

    pub(crate) fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    pub(crate) fn is_shown(&self, session_id: &str) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|s| s.session_id == session_id)
    }

    /// Mounted entries beyond `cap` to suspend: least recently used first, never `keep` or the shown one.
    pub(crate) fn over_cap(&self, cap: usize, keep: &str) -> Vec<String> {
        let mut mounted: Vec<(&String, u64)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.mount.is_some())
            .map(|(id, e)| (id, e.last_used))
            .collect();
        if mounted.len() <= cap {
            return Vec::new();
        }
        mounted.sort_by_key(|(_, used)| *used);
        let excess = mounted.len() - cap;
        mounted
            .into_iter()
            .filter(|(id, _)| id.as_str() != keep && !self.is_shown(id))
            .take(excess)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The URL rules for `session_id`'s mount of `class`: a Local mount reaches the ports any session's
    /// preview may open and every session's servers (as the egress guard decides), never one of `internal`
    /// (Vibe Work's own); a Lan mount its own host. `internal` unknown (`None`, no guard): fail closed, a Local mount
    /// reaches only its own session's listed ports (never "no internal ports").
    pub(crate) fn rule_page(
        &self,
        session_id: &str,
        class: crate::policy::PageClass,
        internal: Option<&[u16]>,
    ) -> vw_netguard::rules::RulePage {
        use vw_netguard::rules::RulePage;
        match class {
            crate::policy::PageClass::Web => RulePage::Web,
            crate::policy::PageClass::Lan => self
                .entries
                .get(session_id)
                .and_then(|e| e.lan_host.clone())
                .map_or(RulePage::Web, |host| RulePage::Lan { host }),
            crate::policy::PageClass::Local => {
                let mut ports: Vec<u16> = match internal {
                    Some(internal) => self
                        .entries
                        .values()
                        .flat_map(|e| e.allowed.iter().copied())
                        .chain(self.server_ports.iter().copied())
                        .filter(|p| !internal.contains(p))
                        .collect(),
                    None => self
                        .entries
                        .get(session_id)
                        .map(|e| e.allowed.clone())
                        .unwrap_or_default(),
                };
                ports.sort_unstable();
                ports.dedup();
                RulePage::Local { ports }
            }
        }
    }

    /// The session of a mount label (a live mount only).
    pub(crate) fn session_of(&self, label: &str) -> Option<&str> {
        self.by_label.get(label).map(String::as_str)
    }
}
