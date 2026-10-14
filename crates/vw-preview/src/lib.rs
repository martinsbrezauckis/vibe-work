//! The preview pane's model: one preview per session, shown in the side
//! panel's Browser tab as a native child webview with its own website data, limited to the session's own loopback
//! servers; screenshots and console entries for the session's agent; downloads into the session folder.
//!
//! Everything here runs without a window: the native engine is behind [`PreviewEngine`] (Tauri implementation in
//! vw-webview; a recording fake in the tests).
//!
//! Depends on: url, png, base64, sha2, tokio, serde. Called by: vw-core-api (commands, agent tools, lifecycle),
//! vw-webview (engine types, storage names, console scripts).
//! Invariants:
//! - A session's browser loads public `http`/`https` pages and plain `http` on the loopback interface on ports of its
//!   own session; private-network addresses only when the user typed them (a Lan page the agent neither reads nor acts
//!   on); never link-local or special addresses, other schemes or the app's own origins ([`UrlPolicy`]). Host names
//!   are checked against DNS on every in-place navigation (`names`). The
//!   request-time layers (the egress guard, the `WebView2` request barrier, macOS content rules; [`GuardLayer`]) are
//!   extra where they run; without them public pages still open.
//! - Public web pages share a dedicated browser profile; local and LAN pages retain per-session storage.
//! - Untrusted page text (titles, console messages, URLs) is length-capped before it leaves this crate.

#[cfg(all(feature = "e2e-script", not(debug_assertions)))]
compile_error!("the e2e-script browser-origin seam is unavailable in release builds");

pub mod accounts;
mod browse;
mod compose;
mod console;
mod downloads;
mod engine;
mod engine_pool;
mod hooks;
mod ids;
mod layout;
mod names;
mod native_click;
mod page_init;
mod policy;
mod service;
mod service_evaluate;
mod shot;
mod state;
mod tabs;
mod url_rules;
mod visibility;

pub use browse::{LoadState, LoopbackGate, SHOTS_BLOCKED, shots_protected};
pub use compose::{PixelRect, canvas, crop, fit, paste};
pub use console::{
    ConsoleEntry, ConsoleLevel, HOST_RING, MAX_RETURNED, MAX_TEXT, PAGE_RING, accessor_name,
    drain_script, init_script, parse_drain,
};
pub use downloads::{
    DEFAULT_MAX_BYTES, DownloadState, DownloadView, clean_name, deliver, staging_file,
};
pub use engine::{
    CheckDone, EngineError, EngineFuture, GuardCheck, GuardLayer, HistoryStep, LaterCheck, MountId,
    MountSpec, PageEvent, PreviewEngine, PreviewHooks, RequestCheck, Storage,
};
pub use engine_pool::{BrowserSaveGuard, EnginePoolSnapshot, effective_browser_cap};
pub use ids::{LABEL_PREFIX, StorageKey, is_preview_label};
pub use layout::{DeviceWidth, MIN_SIDE, PARKED_AREA, Placement, Rect, parked, place};
pub use names::{
    CACHE_CAP, LOOKUP_LIMIT, MAX_LOOKUPS, MAX_QUEUED, MAX_WAITERS, NameResolver, NameStats,
    NameVerdict, RETRY_AFTER, TTL, UI_WAIT, system_resolver,
};
pub use native_click::{NativeClickGuard, NativeClickPoint};
pub use page_init::{page_accessor, page_init_script};
pub use policy::{Destination, PageClass, PortOwner, UrlPolicy, UrlRefusal, loopback_url};
pub use service::{OpenRequest, PreviewConfig, PreviewError, PreviewService};
pub use shot::{
    MAX_BYTES, MAX_WIDTH, Rgba, Shot, decode, decode_limited, downscale, encode, shape, to_srgb,
};
pub use state::{PreviewServerState, PreviewState, PreviewView};
pub use tabs::{PageTab, PageTabKind, PageTabOwner};
pub use visibility::VisibilityReport;

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "preview pane model: per-session previews, URL policy, engine seam";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
