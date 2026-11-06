//! Helper isolation policy and native entry points. Only the helper installs restrictions.
//! Pure policy tests run on every host; native denial probes require their actual OS.
/// Maximum elapsed time for one extraction, including sandbox startup; enforced by the parent.
pub const DEADLINE: std::time::Duration = std::time::Duration::from_secs(35);
#[cfg(windows)]
mod windows;
#[cfg(any(test, windows))]
mod windows_limits;
#[cfg(windows)]
pub use windows::{PREFIX, launch, sweep, verify};
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{resident_bytes, restrict};
#[cfg(any(test, target_os = "macos"))]
mod profile;
