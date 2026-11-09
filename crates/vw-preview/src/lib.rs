//! The preview pane's model: one preview per session, shown in the side
//! panel's Browser tab as native child webviews with their own website data; screenshots and console entries for the session's agent; downloads into the session folder.
//!
//! Everything here runs without a window: the native engine is behind [`PreviewEngine`] (Tauri implementation in
//! vw-webview; a recording fake in the tests).
//!
//! Depends on: url, png, base64, sha2, tokio, serde. Called by: vw-core-api (commands, agent tools, lifecycle),
//! vw-webview (engine types, storage names, console scripts).
//! Invariants:
//! - Browser tabs allow local and public HTTP/HTTPS addresses. Live mount identity, credential and scheme checks
//!   remain; the pinned App preview opens the session's own servers. Backend capture uses a separate fetch policy.
//! - Public web pages share a dedicated browser profile; local and LAN pages retain per-session storage.
//! - Untrusted page text (titles, console messages, URLs) is length-capped before it leaves this crate.

#[cfg(all(feature = "e2e-script", not(debug_assertions)))]
compile_error!("the e2e-script browser-origin seam is unavailable in release builds");

pub mod accounts;
mod audio;
pub use audio::AudioQuiet;
mod act_refusals;
#[cfg(feature = "click-marker")]
mod action_overlay;
pub mod action_tracking;
mod browse;
mod cdp_frames;
mod compose;
mod console;
mod dialogs;
mod windows;
pub use windows::{PageWindowAction, PageWindowInfo};
mod download_tools;
pub use dialogs::DialogAnswer;
mod pointer;
pub use pointer::ClickOptions;
mod downloads;
mod site_cookies;
pub use site_cookies::CookieRequest;
mod engine;
mod engine_pool;
pub use act_refusals::ActRefusal;
mod fetch_policy;
mod frames;
mod hooks;
mod ids;
mod layout;
mod names;
mod native_click;
mod navigation;
mod page_init;
mod person_pointer;
mod placement_gate;
mod policy;
mod popups;
mod preview_error;
mod service;
mod service_evaluate;
pub mod shields;
mod shot;
mod state;
mod tabs;
mod url_rules;
mod visibility;

pub use browse::{LoadState, LoopbackGate, SHOTS_BLOCKED, shots_protected};
pub use cdp_frames::{CDP_EVENTS, CdpDone, CdpFrames, CdpTransport, FramesDone};
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
pub use engine_pool::{BrowserSaveGuard, EnginePoolSnapshot, MemoryProbe, effective_browser_cap};
pub use frames::{
    DETACHED_FRAME, FrameAnswer, UNREADABLE_FRAME, UNREPORTING_FRAME_REFUSAL,
    checked_frame_request, detached_aware, main_answer, main_script, validate_frame_point,
};
pub use ids::{LABEL_PREFIX, StorageKey, is_preview_label};
pub use layout::{DeviceWidth, MIN_SIDE, PARKED_AREA, Placement, Rect, parked, place};
pub use names::{
    CACHE_CAP, LOOKUP_LIMIT, MAX_LOOKUPS, MAX_QUEUED, MAX_WAITERS, NameResolver, NameStats,
    NameVerdict, RETRY_AFTER, TTL, UI_WAIT, system_resolver,
};
pub use native_click::{NativeClickGuard, NativeClickLease, NativeClickPoint};
pub use page_init::{MENU_TOKENS, page_accessor, page_init_script, page_init_script_themed};
pub use person_pointer::{PERSON_BUSY, PERSON_CLICKED, PersonInterval, PersonPointer};
pub use placement_gate::{PlacementGate, for_window_generation};

/// The all-frames document-start script that removes WebRTC (`js/no_peer_connections.js`). The engine adds it
/// for mounts with [`MountSpec::no_peer_connections`].
pub const NO_PEER_CONNECTIONS_SCRIPT: &str = include_str!("js/no_peer_connections.js");
pub use policy::{Destination, PageClass, PortOwner, UrlPolicy, UrlRefusal, loopback_url};
pub use service::{OpenRequest, PreviewConfig, PreviewError, PreviewService};
pub use shot::{
    MAX_BYTES, MAX_WIDTH, Rgba, Shot, decode, decode_limited, downscale, encode, shape, to_srgb,
};
pub use state::{PreviewServerState, PreviewState, PreviewView};
pub use tabs::{PageTab, PageTabKind, PageTabOwner};
pub use visibility::VisibilityReport;
mod capture_block;
pub use capture_block::{CaptureBlock, CaptureRefusal};

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "preview pane model: per-session previews, URL policy, engine seam";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
