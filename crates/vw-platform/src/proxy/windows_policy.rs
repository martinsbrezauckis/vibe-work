//! Windows automatic-proxy selection, independent of `WinHTTP` so Linux tests exercise the same policy.
//! Called by the Windows adapter with native lookup and manual/default fallback closures.
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) const FAILED: &str = "Windows proxy resolution failed.";
const NEGATIVE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Manual,
    Discover,
    PacUrl,
}

impl Mode {
    pub(super) fn from_settings(auto_detect: bool, pac_url: bool) -> Self {
        if pac_url {
            Self::PacUrl
        } else if auto_detect {
            Self::Discover
        } else {
            Self::Manual
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    WinHttp(u32),
    Timeout,
    Internal,
}

impl Failure {
    fn discovery_absent(self) -> bool {
        // WinHTTP SDK codes, kept here so the policy can be tested on non-Windows hosts:
        // AUTODETECTION_FAILED, NAME_NOT_RESOLVED, UNABLE_TO_DOWNLOAD_SCRIPT (includes HTTP 404).
        // WinINet/Chromium fall back to manual settings when automatic initialization times out:
        // https://www.chromium.org/developers/design-documents/network-stack/proxy-settings-fallback/
        // Treat both ERROR_WINHTTP_TIMEOUT (12002) and our bounded callback wait the same way.
        // Script evaluation, authentication and resolver-service errors still fail closed.
        matches!(
            self,
            Self::WinHttp(12180 | 12007 | 12167 | 12002) | Self::Timeout
        )
    }
}

#[derive(Default)]
pub(super) struct Policy {
    absent_until: Mutex<Option<Instant>>,
}

impl Policy {
    pub(super) fn resolve(
        &self,
        mode: Mode,
        automatic: impl FnOnce() -> Result<Vec<String>, Failure>,
        fallback: impl FnOnce() -> Result<Vec<String>, &'static str>,
    ) -> Result<Vec<String>, &'static str> {
        // Only cache absence, never per-URL PAC routes or manual settings. Each call reads fresh
        // IE/default settings; an explicit PAC always runs, even while a negative entry is live.
        // No network watcher is needed: moving to another network costs at most five minutes.
        let mut absent_until = self
            .absent_until
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if mode != Mode::Discover {
            *absent_until = None;
            drop(absent_until);
            return if mode == Mode::Manual {
                fallback()
            } else {
                automatic().map_err(|_| FAILED)
            };
        }
        if absent_until.is_some_and(|until| Instant::now() < until) {
            drop(absent_until);
            return fallback();
        }
        // Do not serialize per-URL PAC evaluation behind this cache: the guard has a 15s deadline
        // shared by queueing and lookup. Concurrent cold misses may each wait once (the guard bounds
        // workers to four); subsequent connections skip WinHTTP for the entire negative TTL.
        *absent_until = None;
        drop(absent_until);
        match automatic() {
            Err(error) if error.discovery_absent() => {
                *self
                    .absent_until
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(Instant::now() + NEGATIVE_TTL);
                fallback()
            }
            result => result.map_err(|_| FAILED),
        }
    }
}

#[cfg(test)]
#[path = "windows_policy_tests.rs"]
mod tests;
