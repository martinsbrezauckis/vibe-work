//! Hardening of the core process, which holds the Mistral key in memory, using a common
//! agent-CLI process-hardening pattern:
//! - Linux: `PR_SET_DUMPABLE 0` (no core dumps; other same-user processes cannot ptrace it or read
//!   its `/proc/<pid>/environ` and memory) and `RLIMIT_CORE 0`;
//! - other Unix (macOS): `RLIMIT_CORE 0`. `PT_DENY_ATTACH` would need our own `unsafe` and also
//!   blocks debuggers; not done (residual risk);
//! - Windows: nothing cheap without `unsafe` (residual risk: same-user processes can read process
//!   memory).
//!
//! Child processes are unaffected: the kernel resets "dumpable" on `execve`.
//!
//! Depends on: rustix (Unix). Called by: the app at start (before any key is loaded).

/// What hardening achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hardening {
    /// Core dumps are off (`RLIMIT_CORE` 0).
    pub core_dumps_off: bool,
    /// Linux: the process is not dumpable (no ptrace or `/proc` reads by same-user processes).
    pub not_dumpable: bool,
}

/// Applies the hardening this platform supports. Never fails: what could not be applied is
/// `false` in the result (the caller logs it).
#[must_use]
pub fn harden_process() -> Hardening {
    #[cfg(unix)]
    let core_dumps_off = {
        use rustix::process::{Resource, Rlimit, setrlimit};
        setrlimit(
            Resource::Core,
            Rlimit {
                current: Some(0),
                maximum: Some(0),
            },
        )
        .is_ok()
    };
    #[cfg(not(unix))]
    let core_dumps_off = false;
    #[cfg(target_os = "linux")]
    let not_dumpable = {
        use rustix::process::{DumpableBehavior, set_dumpable_behavior};
        set_dumpable_behavior(DumpableBehavior::NotDumpable).is_ok()
    };
    #[cfg(not(target_os = "linux"))]
    let not_dumpable = false;
    Hardening {
        core_dumps_off,
        not_dumpable,
    }
}

/// Linux: whether the process is currently not dumpable (for tests and diagnostics).
#[must_use]
pub fn is_not_dumpable() -> bool {
    #[cfg(target_os = "linux")]
    {
        matches!(
            rustix::process::dumpable_behavior(),
            Ok(rustix::process::DumpableBehavior::NotDumpable)
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}
