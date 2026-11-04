//! Bounded video requests and isolated helper jobs. Called by the core's optional video tool.
//! Uses the existing screenshot limits and MCP image envelope; never sends source movie bytes.
pub mod input;
#[cfg(target_os = "linux")]
mod process;
pub mod protocol;
pub mod sandbox;
pub mod scenes;
#[cfg(feature = "helper")]
pub mod worker;
