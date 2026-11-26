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
    if vw_supervisor::adapter_python(server).is_none() {
        return Err(LaunchError::Version(
            vw_supervisor::SupervisorError::UnprovenInterpreter.to_string(),
        ));
    }
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

    #[cfg(unix)]
    #[test]
    fn development_version_check_never_executes_an_unguardable_server() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("vw-dev-version-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let server = dir.join("vibe-app-server");
        let marker = dir.join("executed");
        std::fs::write(
            &server,
            format!("#!/bin/sh\n: > '{}'\nread request\nprintf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"serverInfo\":{{\"version\":\"3.1.5\"}}}}}}'\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = super::check(&server, &dir.join("run"))
            .unwrap_err()
            .to_string();
        assert!(
            !marker.exists(),
            "development check executed an unguardable server"
        );
        assert!(
            error.contains("cannot run with Vibe Work's safety guards"),
            "{error}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
