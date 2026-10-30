//! The local model proxy: `vibe-app-server` never holds the Mistral
//! key. Its provider `api_base` (and the other Mistral base URLs it uses) point
//! at this loopback server; its `MISTRAL_API_KEY` is a **placeholder** that is also the proxy token.
//! The proxy accepts a request only with the current token, only on an exact allowlist of Mistral
//! paths, replaces `Authorization` with the real key from core memory, and streams the answer back
//! unchanged (status, headers, body), so Mistral's own 401/429/5xx reach Vibe as they are.
//! Built-in context compaction omits historical image payloads from that model request. Image
//! requests and compaction calls have response deadlines, while Vibe's stored history keeps its
//! original images.
//!
//! The token is new on every server start: the supervisor fetches the child's `MISTRAL_API_KEY`
//! from [`ProxySecretProvider`] right before it spawns the server (`vw-supervisor/src/launch.rs`),
//! which rotates the token at that moment, so a token from a previous start is refused and a restart
//! can never leave Vibe with a stale one (the token travels as the placeholder key,
//! never in the URL, because Vibe logs URLs).
//!
//! The answer's `x-ratelimit-*` headers (five exact names, numbers only) are published as a
//! [`RateLimits`] snapshot through `ProxyConfig::on_rate_limits`; nothing else is read.
//!
//! the same listener forwards custom MCP servers that need an API key under `/vw-mcp/<binding id>`
//! (`mcp.rs`): Vibe sends the proxy token as `VW_MCP_TOKEN`, the core's resolver supplies the upstream the user
//! confirmed and the key from the vault, so the key never reaches Vibe's process or its config.
//!
//! a connector call that would write to a code forge is answered here, never forwarded (`connectors.rs`).
//!
//! Depends on: axum (server), vw-mistral (HTTPS client), vw-supervisor (`SecretProvider`), tokio.
//! Called by: vw-keys. Invariants:
//! - binds 127.0.0.1 on a port the OS picks;
//! - an MCP request goes only to the upstream its binding names (never an address from the request), and a
//!   redirect from it is refused;
//! - never logs a header value, the key, the token or a query string (path templates and status
//!   codes only);
//! - while no key is available (vault locked, key not set up) a request waits (bounded) and the
//!   owner is told once per waiting request; it is never forwarded with the placeholder.

mod compaction;
mod connector_bodies;
pub mod connectors;
pub mod credentials;
mod key;
mod limits;
mod mcp;
mod reasoning;
mod routes;
mod server;
mod token;

pub use key::{KeyCell, KeyNeed, KeyState};
pub use limits::{LIMITS_EVERY, RateLimits};
pub use mcp::{MCP_PREFIX, McpFuture, McpRefusal, McpResolver, McpResolverSlot, McpTarget};
pub use routes::{ALLOWLIST, RouteSpec, Upstream, match_route};
pub use server::{LimitsHook, ProxyConfig, ProxyError, ProxyHandle, ProxyStats, Upstreams};
pub use token::{PLACEHOLDER_LEN, ProxySecretProvider, TokenSlot, new_placeholder};
