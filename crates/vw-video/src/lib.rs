//! Bounded video requests and isolated helper jobs. Called by the core's optional video tool.
//! Uses the existing screenshot limits and MCP image envelope; never sends source movie bytes.
pub mod input;
mod process;
pub mod protocol;
pub mod sandbox;
pub mod scenes;
mod staging;
#[cfg(feature = "helper")]
pub mod worker;
