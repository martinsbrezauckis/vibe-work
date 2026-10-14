//! The Tools marketplace: the curated catalog of skill
//! packs, MCP servers and plugins; pinned downloads; safe unpacking; plugin assembly in Vibe's Agent Plugins 1.0
//! format; and the installed state in Vibe Work's own Vibe home.
//!
//! - [`Catalog`]: the compiled-in, reviewed catalog ([`catalog::BUILT_IN`]) and its validation.
//! - [`Installer`]: download -> verify (SHA-256 / npm SRI) -> unpack listed members -> assemble -> optional setup step
//!   -> receipt -> move into `plugins/`.
//! - [`Store`]: where installs live, receipts, listing, removal.
//! - [`Fetcher`] / [`HttpFetcher`]: downloads (tests serve a local fake index).
//!
//! Depends on: sha2, flate2, tar, zip, base64, reqwest (vw-mistral's shared client), tokio, serde. Knows nothing about
//! Vibe's protocol: the core sends `plugin/reload` and reads `plugin_catalog/read` itself. Called by: vw-core-api.
//! Invariants: see each module; the overall one is that nothing downloaded is written before its pin matched, and a
//! folder the marketplace did not create is never replaced or removed.

mod archive;
mod assemble;
pub mod catalog;
mod catalog_rules;
mod digest;
mod error;
mod fetch;
mod install;
mod store;

pub use archive::{
    MAX_MEMBER_BYTES, MAX_NPM_MEMBERS, MAX_TOTAL_BYTES, SCAN_LIMITS, ScanLimits, read_listed,
    read_listed_within, read_npm, read_npm_within,
};
pub use assemble::bundled;
pub use catalog::{
    Access, ArchiveFormat, Catalog, Edit, Entry, FileSpec, Kind, McpServerSpec, Member, NpmPackage,
    Pack, Pairing, PluginSpec, PostInstall, RepoSource, Rules, Runtime, RuntimeTarget, Server,
    current_target,
};
pub use digest::{matches_sri_sha512, parse_sri_sha512, sha256_hex};
pub use error::MarketError;
pub use fetch::{Fetcher, Fetching, HttpFetcher, Progress};
pub use install::{Installer, OnStep, Step};
pub use store::{RECEIPT, Receipt, Store, checked_id};
