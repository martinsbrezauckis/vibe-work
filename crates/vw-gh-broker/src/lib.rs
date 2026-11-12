//! Standalone GitHub read broker, called by a trusted app and a credential-free PATH shim.
//! Depends on local IPC, serde, clap and the real GitHub CLI. The broker owns the token;
//! each request is checked before gh starts, and returned bytes have the token redacted.

pub mod entry;
mod local;
mod policy;
mod runner;
mod wire;

pub use runner::Broker;
pub use wire::{Reply, Request, call};

/// Environment variable given to the shim: a local endpoint name, never a credential.
pub const ENDPOINT_ENV: &str = "VW_GH_BROKER_ENDPOINT";

/// Shared instruction for remote-write refusals. With a literal reason, builds a complete static outcome.
/// The hook, connector and GitHub reader use this so only the user is directed to their controls.
#[macro_export]
macro_rules! remote_change_refusal {
    () => {
        "For remote changes, ask the user to use Push or Create PR in the folder's Changes tab, or make the change on the code hosting site."
    };
    ($reason:expr) => {
        concat!($reason, " ", $crate::remote_change_refusal!())
    };
}
