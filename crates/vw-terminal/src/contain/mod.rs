//! Process-tree containment for one shell: everything the shell starts can be found and stopped.
//!
//! Depends on: `unix` (portable-pty's `setsid`: the shell's pid is the session id of its whole tree)
//! or `windows` (a kill-on-close Job Object). Called by: `terminal` (after spawn, on close, when the
//! shell exits by itself, on drop).
//! Invariants:
//! - [`Containment::terminate`] ends with no member left alive when it returns `true`.
//! - [`Containment::kill_now`] never blocks and never fails loudly (it runs in destructors).

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use unix::Containment;
#[cfg(windows)]
pub(crate) use windows::Containment;
