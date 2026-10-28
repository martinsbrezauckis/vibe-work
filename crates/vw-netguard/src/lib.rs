//! The agent browser's network guard (navigation policy): which addresses the built-in browser may reach, and an app-local HTTP
//! proxy that enforces it at request time for every request a browser webview sends (documents and subresources).
//! It is an extra layer on top of the navigation policy (a listener per data folder or store: Linux, macOS 14+, and
//! Windows next to the per-webview request barrier, [`chromium`]); on Linux it is never a condition for loading a
//! page, while macOS and Windows previews that cannot have it are refused (vw-webview). [`rules`] gives the same address table as content rules for engines that filter by URL (macOS).
//!
//! Depends on: tokio, url, getrandom, `serde_json`, hyper-util (the user's proxy rules, [`upstream`]). Called by: the app (binds the proxy at startup), vw-core-api (serves it,
//! answers loopback questions, reads refusals), vw-webview (proxy URL and canary check per webview), vw-host
//! (internal ports), vw-preview (address classes of literal hosts).
//! Invariants:
//! - Public = global unicast only ([`ip::classify`]); a host NAME is refused when any of its DNS answers is not
//!   public, and the proxy connects only to the addresses it checked (no second resolution: DNS rebinding).
//! - This computer only as `127.0.0.1` / `[::1]` / `localhost`, never Vibe Work's own ports, and only when the
//!   session's [`PageContext`] allows the port (Private Network Access idea).
pub mod chromium;
pub mod decide;
pub mod forward;
mod guard;
pub mod head;
pub mod ip;
pub mod resolve;
pub mod rules;
mod serve;
pub mod upstream;
mod ws_origin;

pub use decide::{NoLoopback, Scope, ScopeFuture};
pub use guard::{
    CANARY_SUFFIX, LOG_CAP, LOOPBACK_CANARY_PATH, MAX_CONNECTIONS, NetGuard, PageContext, Refusal,
    SEEN_CAP,
};
pub use resolve::{Resolve, ResolveFuture, SystemResolver};
pub use upstream::Upstream;
