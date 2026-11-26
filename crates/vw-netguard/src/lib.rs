//! The agent browser's network guard (navigation policy): which addresses the built-in browser may reach, and an app-local HTTP
//! proxy that enforces it at request time for every request a browser webview sends (documents and subresources).
//! A separate host-owned listener carries core and Vibe native automatic proxy routes through the same checked-address
//! transport while keeping the browser's address permissions separate.
//! It is an extra layer on top of the navigation policy (a listener per data folder or store: Linux, macOS 14+, and
//! Windows next to the per-webview request barrier, [`chromium`]); on Linux it is never a condition for loading a
//! page, while macOS and Windows previews that cannot have it are refused (vw-webview). [`rules`] gives the same address table as content rules for engines that filter by URL (macOS).
//!
//! Depends on: tokio, url, getrandom, `serde_json`, hyper-util (the user's proxy rules, [`upstream`]). Called by: the app (binds the proxy at startup), vw-core-api (serves it,
//! answers loopback questions, reads refusals), vw-webview (proxy URL and canary check per webview), vw-host
//! (internal ports), vw-preview (address classes of literal hosts).
//! Invariants:
//! - Browser public targets are global unicast only ([`ip::classify`]); local targets need their browser scope's
//!   permission. All connections use checked addresses (no second resolution: DNS rebinding).
//! - Browser access to this computer needs the session's [`PageContext`] permission. Its own interface addresses
//!   ([`own`]) count as this computer too. Vibe Work's own ports are refused.
//! - The host-owned native automatic relay can reach checked local addresses for schemes without a manual proxy.
//!   The ordinary shared listener and manual routes keep their stricter policy.
//! - Vibe Work's own ports include every guard's listeners in the process ([`listeners`]), not only the asking guard's.
pub mod chromium;
#[cfg(feature = "proxy-transports")]
mod connect_protocol;
#[cfg(feature = "proxy-transports")]
mod connect_tunnel;
mod credential;
pub mod decide;
pub mod forward;
mod guard;
pub mod head;
pub mod ip;
pub mod listeners;
pub mod own;
pub mod resolve;
pub mod rules;
mod serve;
mod stream;
#[cfg(not(feature = "proxy-transports"))]
pub mod upstream;
#[cfg(feature = "proxy-transports")]
#[path = "proxy/mod.rs"]
pub mod upstream;
mod ws_origin;

pub use decide::{LocalRequest, NoLoopback, Scope, ScopeFuture};
#[cfg(feature = "proxy-transports")]
pub use guard::ServiceStreamError;
pub use guard::{
    CANARY_SUFFIX, LOG_CAP, LOOPBACK_CANARY_PATH, MAX_CONNECTIONS, NetGuard, PageContext, Refusal,
    SEEN_CAP,
};
pub use resolve::{Resolve, ResolveFuture, SystemResolver};
pub use upstream::Upstream;
