//! "Use my existing install": finds `vibe-app-server` executables the user already has (PATH and
//! the known install locations, through vw-supervisor's discovery: absolute, canonical, `.exe` only on
//! Windows) and reads each one's version from its `initialize` answer (2.26.0 has no `--version`).
//!
//! The probe runs the user's binary for a few seconds with a throwaway `VIBE_HOME` and home under `probe_dir`
//! (never `~/.vibe`), no key, the allowlisted environment, and stdin closed after the answer.
//!
//! Depends on: vw-supervisor (discovery, `default_initialize_params`), tokio. Called by: [`crate::manager`].

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use vw_supervisor::{Discovery, canonical_executable};

use crate::view::ExistingView;

/// Longest a probe may take.
pub const PROBE_LIMIT: Duration = Duration::from_secs(25);
/// Candidates probed at most.
pub const MAX_PROBES: usize = 4;

/// The reason an install of another version cannot be chosen.
#[must_use]
pub fn version_warning(found: &str, pinned: &str) -> String {
    format!("This install is Vibe {found}; Vibe Work needs Vibe {pinned}.")
}

/// True when the canonical candidate `path` lies in Vibe Work's own install folder `own`, which is skipped (it is
/// not the user's existing install).
///
/// `path` comes from `canonical_executable` (links resolved; on Windows long names and the on-disk case, without the
/// verbatim prefix), while `own` is the folder as configured: through a symlinked folder, or on Windows in another
/// spelling, the plain comparison misses. Then both are compared in their canonical forms (`canonicalize`); a side
/// that cannot be canonicalized compares as written only.
fn in_own_install(path: &Path, own: &Path) -> bool {
    path.starts_with(own)
        || matches!(
            (path.canonicalize(), own.canonicalize()),
            (Ok(path), Ok(own)) if path.starts_with(&own)
        )
}

/// Every distinct existing server, with its version (or `None` when it did not answer).
pub async fn find(
    discovery: &Discovery,
    probe_dir: &Path,
    parent_env: &[(OsString, OsString)],
    pinned: &str,
    exclude: &Path,
) -> Vec<ExistingView> {
    let mut seen: Vec<PathBuf> = Vec::new();
    for candidate in discovery.candidates() {
        if let Some(path) = canonical_executable(&candidate)
            && !in_own_install(&path, exclude)
            && !seen.contains(&path)
        {
            seen.push(path);
        }
        if seen.len() == MAX_PROBES {
            break;
        }
    }
    let mut out = Vec::new();
    for (index, path) in seen.into_iter().enumerate() {
        let home = probe_dir.join(format!("probe-{index}"));
        let version = probe(&path, &home, parent_env).await;
        let supported = version.as_deref() == Some(pinned);
        let warning = match &version {
            Some(v) if v != pinned => Some(version_warning(v, pinned)),
            None => Some(
                "This program did not answer like Vibe's app server; it cannot be used.".to_owned(),
            ),
            _ => None,
        };
        out.push(ExistingView {
            path: path.display().to_string(),
            version,
            supported,
            warning,
        });
    }
    out
}

/// Starts `program`, sends `initialize`, returns `serverInfo.version`. `None` on any failure or timeout.
pub async fn probe(
    program: &Path,
    home: &Path,
    parent_env: &[(OsString, OsString)],
) -> Option<String> {
    probe_with_args(program, &[], home, parent_env).await
}

/// The same bounded initialize probe through a development recording wrapper's arguments.
/// The wrapper must transparently launch the real server; it never supplies a version itself.
pub async fn probe_with_args(
    program: &Path,
    args: &[OsString],
    home: &Path,
    parent_env: &[(OsString, OsString)],
) -> Option<String> {
    tokio::time::timeout(PROBE_LIMIT, probe_inner(program, args, home, parent_env))
        .await
        .ok()
        .flatten()
}

async fn probe_inner(
    program: &Path,
    args: &[OsString],
    home: &Path,
    parent_env: &[(OsString, OsString)],
) -> Option<String> {
    let vibe_home = home.join("vibe-home");
    std::fs::create_dir_all(&vibe_home).ok()?;
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let env = crate::steps::environment(
        parent_env.iter().cloned(),
        &[
            ("VIBE_HOME".to_owned(), vibe_home.into_os_string()),
            (home_var.to_owned(), home.as_os_str().to_owned()),
            ("PYTHONUNBUFFERED".to_owned(), OsString::from("1")),
        ],
    );
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(env)
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
    let mut child = crate::steps::spawn_with_retry(|| command.spawn())
        .await
        .ok()?;
    let params = serde_json::to_value(vw_supervisor::default_initialize_params()).ok()?;
    let request =
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": params});
    let mut stdin = child.stdin.take()?;
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .ok()?;
    stdin.flush().await.ok()?;
    let mut lines = BufReader::new(child.stdout.take()?).lines();
    let mut version = None;
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(frame) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if frame.get("id") == Some(&serde_json::json!(1)) {
            version = frame
                .pointer("/result/serverInfo/version")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            break;
        }
    }
    drop(stdin);
    if tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
    version
}

#[cfg(test)]
mod tests {
    use super::in_own_install;
    use std::path::{Path, PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vw-own-install-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("own").join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("other").join("bin")).unwrap();
        std::fs::write(dir.join("own").join("bin").join("server"), b"x").unwrap();
        std::fs::write(dir.join("other").join("bin").join("server"), b"x").unwrap();
        dir
    }

    /// Candidates arrive canonical (links resolved); Vibe Work's own install folder may be configured through a
    /// symlinked folder (a linked home, macOS `/var`). Its server is still its own; another folder's is not.
    #[cfg(unix)]
    #[test]
    fn own_install_through_a_linked_folder_is_recognised() {
        let base = scratch("linked");
        let linked = base.join("linked-own");
        std::os::unix::fs::symlink(base.join("own"), &linked).unwrap();
        let own_server = base.join("own/bin/server").canonicalize().unwrap();
        let other_server = base.join("other/bin/server").canonicalize().unwrap();
        assert!(in_own_install(&own_server, &base.join("own")));
        assert!(in_own_install(&own_server, &linked));
        assert!(!in_own_install(&other_server, &linked));
        assert!(!in_own_install(
            &other_server,
            Path::new("/nonexistent-own-install")
        ));
        std::fs::remove_dir_all(base).unwrap();
    }

    /// The candidate is canonical without the verbatim prefix (`C:\...`, long names, on-disk case); the own folder
    /// may be written in another case, with `/` or verbatim. All name the same install.
    #[cfg(windows)]
    #[test]
    fn own_install_matches_every_windows_spelling() {
        let base = scratch("spellings");
        let verbatim = base.join("own").canonicalize().unwrap();
        let plain = PathBuf::from(verbatim.to_string_lossy().trim_start_matches(r"\\?\"));
        let server = plain.join("bin").join("server");
        let other = PathBuf::from(
            base.join("other")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .trim_start_matches(r"\\?\"),
        )
        .join("bin")
        .join("server");
        for own in [
            base.join("own"),
            verbatim.clone(),
            plain.clone(),
            PathBuf::from(plain.to_string_lossy().replace('\\', "/")),
            PathBuf::from(plain.to_string_lossy().to_uppercase()),
            PathBuf::from(plain.to_string_lossy().to_lowercase()),
        ] {
            assert!(in_own_install(&server, &own), "own {}", own.display());
            assert!(!in_own_install(&other, &own), "own {}", own.display());
        }
        assert!(!in_own_install(
            &server,
            Path::new(r"C:\nonexistent-own-install")
        ));
        std::fs::remove_dir_all(base).unwrap();
    }
}
