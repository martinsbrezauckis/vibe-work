//! Shared synthetic fixtures for public Rust integration tests.
#![allow(dead_code, unreachable_pub)]

pub mod channel_rig;
pub mod closed_port;
pub mod core;
pub mod fake_mistral;
pub mod gate;
pub mod host_fakes;
pub mod http;
pub mod launchers;
pub mod logs;
pub mod process_fault;
pub mod processes;
pub mod real_vibe;
pub use real_vibe as shared_real_vibe;
pub mod revoke;
pub mod scripted_model;
pub mod scripted_platform;
pub mod scripted_registry;
pub mod seed_sessions;
pub mod subagents_rig;
pub mod supervisor_rig;
pub mod support_rig;
pub mod update_rig;
pub mod vault;
pub mod vibe_rig;
