//! Installs Vibe for the user at setup: Vibe Work's installers ship no Vibe;
//! with the user's consent the app installs the **pinned** `mistral-vibe` from Mistral's official channel
//! (`PyPI`) into an **app-private** folder, so the version always matches the generated protocol types and the
//! user's own tools and PATH are never touched. Or the user picks an existing install of theirs.
//!
//! Why not bundle: we don't redistribute Vibe (Mistral Commercial Terms §2.2(i)); the user's machine gets it from
//! the official channel. Pattern: ShellX's agent CLI setup (`src-tauri/src/agent_cli_setup.rs`: recipe with the
//! official source, a confirmation showing what will run and its SHA-256, a time limit, the output tail).
//!
//! Modules: [`pin`] (embedded, generated pin), [`manager`] (state machine, consent), `run` (the install task),
//! [`fetch`] (checksum-verified download), [`steps`] (contained uv processes), [`existing`] (other installs),
//! [`layout`] (folders and records), [`view`] (what the UI and the Debug API see); [`support`] (the
//! signed supported-Vibe manifest; its rollback record in `seen`) and [`update`] (its check and the consent-bound
//! Vibe update).
//!
//! Depends on: vw-mistral (HTTPS client), vw-supervisor (discovery, environment allowlist, initialize params).
//! Called by: vw-core-api (`vibe` commands, the deferred server start), the app, the private tests.

#![forbid(unsafe_code)]

pub mod existing;
pub mod fetch;
pub mod layout;
pub mod manager;
pub mod pin;
mod run;
mod seen;
pub mod steps;
pub mod support;
pub mod update;
pub mod view;

pub use manager::{InstallError, InstallerConfig, Limits, OnReady, VibeInstaller};
pub use pin::{PinError, Sources, UvAsset, this_target};
pub use support::{Manifest, SupportError, SupportedVibe};
pub use update::SupportConfig;
pub use view::{
    ExistingView, FailKind, InstallOffer, InstallPhase, InstallSource, InstallView, SourceView,
    VibeUpdatePhase, VibeUpdateView,
};
