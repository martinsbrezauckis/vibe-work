//! Checks the development server with the product installer's initialize probe.
//! Called by `DevLaunch` before starting the supervisor; only disposable probe homes are written.

use std::path::{Path, PathBuf};

use crate::launch::LaunchError;

pub(crate) fn pinned() -> Result<String, LaunchError> {
    vw_vibe_install::Sources::pinned_version().map_err(|e| LaunchError::Version(e.to_string()))
}

pub(crate) fn runtime(repo: &Path) -> Result<PathBuf, LaunchError> {
    Ok(repo
        .join("data")
        .join(format!("vibe-{}", pinned()?))
        .join("venv"))
}

pub(crate) fn check(server: &Path, run: &Path) -> Result<(), LaunchError> {
    let expected = pinned()?;
    let server = server.to_owned();
    let home = run.join("version-probe");
    let args = std::env::var("VW_DEV_SERVER_ARGS")
        .unwrap_or_default()
        .split_whitespace()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    let parent = std::env::vars_os().collect::<Vec<_>>();
    // DevLaunch is also called from synchronous Tauri setup and from async test runtimes.
    // A separate thread permits the same bounded async probe in both callers.
    let found = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?
            .block_on(vw_vibe_install::existing::probe_with_args(
                &server, &args, &home, &parent,
            ))
    })
    .join()
    .ok()
    .flatten();
    verify(found.as_deref(), &expected)
}

fn verify(found: Option<&str>, expected: &str) -> Result<(), LaunchError> {
    match found {
        Some(found) if found == expected => Ok(()),
        Some(found) => Err(LaunchError::Version(
            vw_vibe_install::existing::version_warning(found, expected),
        )),
        None => Err(LaunchError::Version(format!(
            "The development server did not report a Vibe version; Vibe Work needs Vibe {expected}. Check VW_DEV_SERVER_BIN."
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::verify;

    #[test]
    fn refuses_old_or_unreadable_versions() {
        assert!(verify(Some("3.0.0"), "3.0.0").is_ok());
        let old = verify(Some("2.26.1"), "3.0.0").unwrap_err().to_string();
        assert!(old.contains("2.26.1") && old.contains("3.0.0"));
        assert!(
            verify(None, "3.0.0")
                .unwrap_err()
                .to_string()
                .contains("did not report")
        );
    }
}
