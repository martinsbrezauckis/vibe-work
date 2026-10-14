//! The user's project groups: a project is a **named group the user owns**, not a
//! folder. It has an optional main folder (whose handbook the Project tab and the Projects overview show), a place
//! in the user's order, and members: Vibe sessions filed into it. A session keeps working in its own folder; the
//! grouping is Vibe Work's own data and never touches Vibe's sessions.
//!
//! - [`State`]: the pure model and every rule: create, rename, delete (members become unfiled, never deleted),
//!   move, place, main folder, first-sight filing ([`State::file_new`], [`State::file_seen`]) and the reconcile with
//!   Vibe's full session list ([`State::sweep`]: a member is dropped only when two sweeps at least
//!   [`DROP_AFTER_MS`] apart both missed it).
//! - [`ProjectStore`]: the state in one user-only JSON file in app data, replaced atomically after every change; a
//!   file that does not parse is moved aside, never overwritten. Without a folder it keeps the state in memory.
//!
//! Depends on: vw-platform (`ensure_private_dir`, `replace_private`, `set_aside`), serde, getrandom. Knows nothing
//! about Vibe's protocol. Called by: vw-core-api (`projects` command, session filing hooks, Pages folder choice).
//! Invariants:
//! - Deleting a project never removes a member: its sessions become "not in a project" (`None`).
//! - Memory never runs ahead of disk: a change is written first and swapped in only when the write succeeded.
//! - Names are cleaned (control characters dropped, whitespace collapsed, 1-120 characters).

mod model;
mod names;
mod state;
mod store;

pub use model::{
    Hint, MAX_MEMBERS, MAX_NAME_CHARS, MAX_PROJECTS, NewSession, Project, ProjectsError, Seen,
    Snapshot, SweepOutcome,
};
pub use names::{clean_name, folder_label, normalize_folder, same_folder};
pub use state::{DROP_AFTER_MS, MAX_MOVE, State};
pub use store::{FILE_NAME, ProjectStore};

/// Milliseconds since the Unix epoch (0 when the clock is before it).
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
