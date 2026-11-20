//! Bounded native request observations, owned by one webview and discarded on its next document.
//! Engine adapters record facts only; browser tools apply Full-access header checks and output redaction.
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use tauri::{Webview, Wry};
use vw_preview::EngineError;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(windows)]
mod windows;
pub(crate) type Shared = Arc<Mutex<Log>>;
#[derive(Default)]
pub(crate) struct Log {
    /// Declarative closed shadow roots (`vw_preview::declarative_closed_source`) in the HTML this webview received
    /// for its current page: sticky until a new main document from the network commits (`Sources`).
    sources: Sources,
    rows: VecDeque<Value>,
    bytes: usize,
    prefix: String,
    next: u64,
    dropped: usize,
    unavailable: Option<String>,
}
/// Whether HTML the webview received for its current page may attach a closed declarative shadow root. Not keyed by
/// address: a page cannot clear it by fetching innocent HTML at its own address, by many other responses, or by
/// moving its address with `history.pushState`. Any HTML response counts (a page's own fetch of such HTML only makes
/// it stricter); one still arriving counts until checked. Only a new main document loaded from the network clears
/// it (a page restored from the back-forward cache loads nothing, so the old state stays).
#[derive(Default)]
pub(crate) struct Sources {
    declarative: bool,
    arriving: u32,
    /// The main resource of the load in progress came from the network.
    fresh: bool,
    /// The state of pages left, by their address when left, for a back-forward restore (bounded; newest last).
    left: VecDeque<(String, bool)>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Sources {
    /// A main-frame load started, leaving the page at `address`.
    pub(super) fn load_started(&mut self, address: Option<String>) {
        self.fresh = false;
        if let Some(address) = address {
            // Two pages left at one address keep the stricter state: a later clear page there cannot clear it.
            let mut state = self.declarative();
            self.left.retain(|(kept, earlier)| {
                let same = *kept == address;
                state |= same && *earlier;
                !same
            });
            self.left.push_back((address, state));
            while self.left.len() > 64 {
                self.left.pop_front();
            }
        }
    }
    /// The load's main resource was requested from the network.
    pub(super) fn main_requested(&mut self) {
        self.fresh = true;
    }
    /// The main frame committed its document at `address`: a fresh one starts clear (HTML still arriving, its own
    /// source included, keeps counting until checked); a restored one (no main resource from the network) gets the
    /// state it had when left, and counts as declarative when that is not known.
    pub(super) fn committed(&mut self, address: Option<&str>) {
        self.declarative = if self.fresh {
            false
        } else {
            address
                .and_then(|address| self.left.iter().rev().find(|(kept, _)| kept == address))
                .is_none_or(|(_, state)| *state)
        };
    }
    /// HTML whose arrival was not counted was checked (`verdict`).
    pub(super) fn checked(&mut self, verdict: bool) {
        self.declarative |= verdict;
    }
    /// An HTML response began arriving.
    pub(super) fn arriving(&mut self) {
        self.arriving = self.arriving.saturating_add(1);
    }
    /// That HTML arrived (`Some(verdict)`), was cancelled (`Some(false)`: its document is gone) or failed
    /// (`None`: what was parsed may hold one).
    pub(super) fn arrived(&mut self, declarative: Option<bool>) {
        self.arriving = self.arriving.saturating_sub(1);
        self.declarative |= declarative.unwrap_or(true);
    }
    pub(crate) fn declarative(&self) -> bool {
        self.declarative || self.arriving > 0
    }
}

pub(super) fn lock(log: &Shared) -> MutexGuard<'_, Log> {
    log.lock().unwrap_or_else(PoisonError::into_inner)
}
impl Log {
    pub(super) fn reset(&mut self) {
        self.rows.clear();
        self.bytes = 0;
        self.dropped = 0;
    }
    pub(super) fn start(&mut self, mut row: Value) -> String {
        self.next += 1;
        let id = format!("req-{}-{}", self.prefix, self.next);
        row["requestId"] = json!(id);
        self.bytes += row.to_string().len();
        self.rows.push_back(row);
        self.trim();
        id
    }
    pub(super) fn update(&mut self, id: &str, fields: Value) {
        if let Some(row) = self.rows.iter_mut().find(|r| r["requestId"] == id)
            && let (Some(row), Value::Object(fields)) = (row.as_object_mut(), fields)
        {
            self.bytes = self
                .bytes
                .saturating_sub(serde_json::to_vec(&row).map_or(0, |v| v.len()));
            row.extend(fields);
            self.bytes += serde_json::to_vec(&row).map_or(0, |v| v.len());
        }
        self.trim();
    }
    fn trim(&mut self) {
        while self.rows.len() > 256 || self.bytes > 2 * 1024 * 1024 {
            if let Some(row) = self.rows.pop_front() {
                self.bytes = self.bytes.saturating_sub(row.to_string().len());
            }
            self.dropped += 1;
        }
    }
    /// The page's declarative-root state (Linux resource events).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(super) fn sources(&mut self) -> &mut Sources {
        &mut self.sources
    }
    /// Whether the current page may hold a closed declarative shadow root (`Sources::declarative`).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn declarative(&self) -> bool {
        self.sources.declarative()
    }
    pub(crate) fn snapshot(&self) -> Result<Value, EngineError> {
        if let Some(reason) = &self.unavailable {
            return Err(EngineError::Unsupported(reason.clone()));
        }
        Ok(
            json!({"requests":self.rows,"dropped":self.dropped,"coverage":"Native requests observed in this webview since document load; redirect chains and cross-process child requests may be incomplete."}),
        )
    }
}
pub(crate) async fn install(
    view: &Webview<Wry>,
    label: &str,
    hooks: Arc<dyn vw_preview::PreviewHooks>,
) -> Shared {
    let log = Arc::new(Mutex::new(Log {
        prefix: vw_preview::accessor_name().replace("__vw_console_", ""),
        ..Log::default()
    }));
    #[cfg(any(target_os = "linux", windows, target_os = "macos"))]
    let result = {
        let records = log.clone();
        #[cfg(target_os = "macos")]
        let label = label.to_owned();
        #[cfg(not(target_os = "macos"))]
        let _ = (label, &hooks);
        crate::platform::with_native(
            view,
            "watching page requests",
            std::time::Duration::from_secs(5),
            move |native, reply| {
                #[cfg(target_os = "linux")]
                {
                    linux::install(&native.inner(), records);
                    let _ = reply.send(Ok(()));
                }
                #[cfg(target_os = "macos")]
                {
                    let _ = reply.send(macos::install(native.inner(), records, &label, hooks));
                }
                #[cfg(windows)]
                windows::install(
                    &native.controller(),
                    records,
                    Box::new(move |result| {
                        let _ = reply.send(result);
                    }),
                );
            },
        )
        .await
    };
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    let result: Result<(), EngineError> = {
        let _ = view;
        Err(EngineError::Unsupported("This native engine has no public complete request/header event API; use page.errors for resource failures or inspect the page in its developer tools.".into()))
    };
    if let Err(error) = result {
        lock(&log).unavailable = Some(error.to_string());
    }
    log
}

#[cfg(test)]
mod source_tests {
    use super::Sources;

    #[test]
    fn declarative_html_stays_until_a_fresh_main_document_commits() {
        let mut sources = Sources::default();
        assert!(!sources.declarative());
        sources.arriving();
        assert!(sources.declarative(), "HTML still arriving counts");
        sources.arrived(Some(false));
        assert!(!sources.declarative());
        sources.arriving();
        sources.arrived(Some(true));
        // Innocent HTML afterwards (a page fetching its own address again) does not clear it.
        sources.arriving();
        sources.arrived(Some(false));
        assert!(sources.declarative());
        // Leave it for a fresh page: that one starts clear once its own source has arrived.
        sources.load_started(Some("https://a.example/".into()));
        sources.main_requested();
        sources.arriving();
        sources.committed(Some("https://b.example/"));
        assert!(sources.declarative(), "the new document is still arriving");
        sources.arrived(Some(false));
        assert!(!sources.declarative());
        // Back to the first page from the back-forward cache: its own state returns; an unknown restore counts.
        sources.load_started(Some("https://b.example/".into()));
        sources.committed(Some("https://a.example/"));
        assert!(sources.declarative(), "the restored page held one");
        sources.load_started(Some("https://a.example/".into()));
        sources.committed(Some("https://b.example/"));
        assert!(!sources.declarative(), "the restored page was clear");
        // A clear page left later at the declarative page's address does not clear that address.
        sources.load_started(Some("https://a.example/".into()));
        sources.committed(Some("https://a.example/"));
        assert!(sources.declarative());
        sources.load_started(Some("https://b.example/".into()));
        sources.committed(Some("https://b.example/"));
        sources.load_started(None);
        sources.committed(Some("https://never.example/"));
        assert!(sources.declarative(), "a restore of an unknown page");
        sources.load_started(None);
        sources.main_requested();
        sources.committed(None);
        // A response that failed after it began counts.
        sources.arriving();
        sources.arrived(None);
        assert!(sources.declarative());
    }
}
