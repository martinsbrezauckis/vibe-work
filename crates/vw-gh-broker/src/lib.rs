//! Standalone GitHub read broker, called by a trusted app and a credential-free PATH shim.
//! Depends on local IPC, serde, clap and the real GitHub CLI. The broker owns the token;
//! each request is checked before gh starts, and returned bytes have the token redacted.

mod local;
mod policy;
mod runner;
mod wire;

pub use runner::Broker;
pub use wire::{Reply, Request, call};

/// Environment variable given to the shim: a local endpoint name, never a credential.
pub const ENDPOINT_ENV: &str = "VW_GH_BROKER_ENDPOINT";
