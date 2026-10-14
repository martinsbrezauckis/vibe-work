//! The outer git loop of the Changes tab: the branch bar's facts, the user's own Commit, Push and pull request,
//! the PR card's data, and "Bring into main" for a session copy without a forge.
//!
//! This crate is the git and forge side without Vibe's protocol; the core (`vw-core-api` `git` command) owns
//! the confirmations, the session facts and the events.
//!
//! Modules: [`state`] (branch, upstream, base; read only), [`programs`] (the repository-program check before
//! git touches the working tree or the network), [`commit`], [`push`], [`forge`] (remote addresses and forge
//! pages), [`message`] (commit style, model prompt, fallback, PR text and template), [`gh`] (GitHub's CLI),
//! [`pr`] (the card's data and the prepared messages), [`checklog`] (a failing check's lines for the prepared message,
//! ), [`merge`] (bring a copy into its base).
//!
//! Depends on: vw-changes (the one hardened git runner, with its `Worktree` and `Publish` profiles), serde,
//! url. Called by: vw-core-api.
//! Invariants:
//! - Every git call goes through `vw_changes::git::Call`; only `commit`, `merge` and `push` use the wider
//!   profiles, each after [`programs::check`].
//! - Nothing here reaches the network except [`push::push`] and [`gh::Gh`] calls, and the core makes those only
//!   for a user's action (a press, or the PR card's refresh while the session is shown).
//! - No forge token is stored or read by this crate.

pub mod checklog;
pub mod commit;
mod error;
pub mod forge;
pub mod gh;
pub mod hooks;
pub mod merge;
pub mod message;
pub mod names;
pub mod pr;
pub mod programs;
pub mod push;
mod quoted;
pub mod state;

pub use error::{GitError, clip};

/// One-line statement of what this crate is responsible for.
pub const CRATE_ROLE: &str = "the outer git loop: branch bar, commit, push, pull requests, PR card, bring a copy into its base";
