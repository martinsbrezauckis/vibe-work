//! Browser-local macOS setup failures shared by Settings and agent status. Own-state persistence survives
//! restart; the in-memory copy also works if the app's own folder cannot be written. A successful retry clears it.
use super::{ChromeControlConfig, Path};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
};

const BLOCKED: &str = "macOS blocked access";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Problem {
    Access,
    Registration,
}

/// Last observed macOS setup refusals, shared by cloned setup configurations.
#[derive(Clone, Default)]
pub struct SetupProblems(Arc<Mutex<BTreeMap<String, Option<Problem>>>>);

fn path(state: &Path, browser: &str) -> std::path::PathBuf {
    state.join(if browser == "Microsoft Edge" {
        "edge-setup-problem.json"
    } else {
        "chrome-setup-problem.json"
    })
}

pub(super) fn io_message(config: &ChromeControlConfig, error: &std::io::Error) -> String {
    if config.is_macos() && error.kind() == std::io::ErrorKind::PermissionDenied {
        BLOCKED.into()
    } else {
        error.to_string()
    }
}

pub(super) fn blocked(config: &ChromeControlConfig, state: Option<&Path>, browser: &str) -> bool {
    observed(config, state, browser) == Some(Problem::Access)
}

pub(super) fn repair_failed(config: &ChromeControlConfig, state: &Path, browser: &str) -> bool {
    observed(config, Some(state), browser) == Some(Problem::Registration)
}

fn observed(config: &ChromeControlConfig, state: Option<&Path>, browser: &str) -> Option<Problem> {
    if !config.is_macos() {
        return None;
    }
    let cached = config
        .setup_problems
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(browser)
        .copied();
    cached.unwrap_or_else(|| {
        state.and_then(|state| {
            match super::super::probe::read_regular(&path(state, browser), 32).as_deref() {
                Some(b"\"access\"") => Some(Problem::Access),
                Some(b"\"registration\"") => Some(Problem::Registration),
                _ => None,
            }
        })
    })
}

pub(super) fn finish<T>(
    config: &ChromeControlConfig,
    state: &Path,
    browser: &str,
    result: Result<T, String>,
) -> Result<T, String> {
    if !config.is_macos() {
        return result;
    }
    let denied = result
        .as_ref()
        .err()
        .is_some_and(|error| error.starts_with(BLOCKED));
    let problem = if denied {
        Some(Problem::Access)
    } else if result.is_err() && super::registered(config, Some(state), browser) {
        Some(Problem::Registration)
    } else {
        None
    };
    {
        config
            .setup_problems
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(browser.into(), problem);
        if let Some(problem) = problem {
            // This is our own state directory, never a browser directory. Retain the diagnosis even after rollback.
            if vw_platform::ensure_private_dir(state).is_ok() {
                let bytes: &[u8] = match problem {
                    Problem::Access => b"\"access\"",
                    Problem::Registration => b"\"registration\"",
                };
                let file = path(state, browser);
                let _ = std::fs::remove_file(&file);
                let _ = vw_platform::write_private_new(&file, bytes);
            }
        } else {
            let _ = std::fs::remove_file(path(state, browser));
        }
    }
    if denied {
        Err(message(browser))
    } else {
        result
    }
}

pub(in crate::chrome_control) fn message(browser: &str) -> String {
    let short = if browser == "Microsoft Edge" {
        "Edge"
    } else {
        "Chrome"
    };
    format!(
        "{BLOCKED}. In System Settings > Privacy & Security > Files & Folders > Vibe Work, allow {browser} if listed, then retry Connect {short}."
    )
}

pub(super) fn clear(config: &ChromeControlConfig) {
    config
        .setup_problems
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}
