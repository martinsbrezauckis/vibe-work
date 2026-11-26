//! Rechecks user-owned installs before launch and replaces an older app-owned runtime after an app update.
//! Called by the core before launch and after attaching the install progress/ready hooks.
use crate::{InstallError, InstallPhase, InstallSource, VibeInstaller, layout::Layout};

/// A new probe home under `probe_dir` for one start check, never a folder an earlier run made: on Windows a folder an
/// elevated run created can be unusable for the normal account, and the remembered Vibe then ends before it answers.
pub(crate) fn fresh_saved_home(probe_dir: &std::path::Path) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    probe_dir.join(format!(
        "saved-{}-{stamp}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

impl VibeInstaller {
    /// Reprobe a remembered user install: it may have changed since the user selected it.
    /// An unsupported executable remains a setup choice with a plain explanation and is never launched.
    pub async fn check_saved_existing(&self) {
        let Some(layout) = &self.inner.layout else {
            return;
        };
        let Some(choice) = layout.choice() else {
            return;
        };
        let (version, warning) = if let Some(warning) =
            crate::existing::interpreter_warning(&choice.path)
        {
            (None, Some(warning))
        } else {
            let home = fresh_saved_home(&layout.probe_dir());
            let result = crate::existing::probe_checked(
                crate::existing::SAVED_PROBE_LIMIT,
                &choice.path,
                &[],
                &home,
                &self.inner.config.parent_env,
            )
            .await;
            // Best effort: a home that cannot be removed now is only a few small files.
            if let Err(error) = vw_platform::remove_owned_tree(&layout.root, &home) {
                tracing::debug!(%error, "the start check's probe home stays");
            }
            if let Err(failure) = &result {
                tracing::warn!(server = %choice.path.display(), ?failure, "the remembered Vibe gave no version");
            }
            let warning = crate::existing::saved_warning(
                &result,
                crate::existing::SAVED_PROBE_LIMIT,
                &self.inner.pinned,
            );
            (result.ok(), warning)
        };
        let supported = version.as_deref() == Some(&self.inner.pinned) && warning.is_none();
        let mut state = self.inner.lock();
        if supported {
            state.phase = InstallPhase::Installed {
                version: self.inner.pinned.clone(),
                server: choice.path.display().to_string(),
                source: InstallSource::Existing,
                disk_bytes: None,
                install_ms: None,
                warning: None,
            };
        } else {
            state.phase = InstallPhase::NotInstalled {};
            state.existing = vec![crate::ExistingView {
                path: choice.path.display().to_string(),
                version,
                supported,
                warning,
            }];
            state.checked_existing = true;
        }
        self.inner.publish(&state);
    }

    /// Installing an app update also updates its previously installed private runtime.
    /// Fresh installs and the user's own installs still use setup; failed upgrades offer a retry.
    ///
    /// # Errors
    /// The same source/offer errors as setup, before any background install is started.
    pub fn upgrade_previous_private(&self) -> Result<bool, InstallError> {
        let layout = self.layout()?;
        if layout.choice().is_some()
            || !matches!(self.inner.lock().phase, InstallPhase::NotInstalled {})
        {
            return Ok(false);
        }
        // The newest finished older install: the one this update replaces (named on the setup screen).
        let previous = std::fs::read_dir(&layout.root)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|version| {
                crate::support::compare_versions(version, &self.inner.pinned)
                    == Some(std::cmp::Ordering::Less)
            })
            .filter_map(|version| Layout::new(layout.root.clone(), &version))
            .filter(|previous| previous.installed().is_some())
            .map(|previous| previous.version)
            .max_by(|a, b| {
                crate::support::compare_versions(a, b).unwrap_or(std::cmp::Ordering::Equal)
            });
        let Some(previous) = previous else {
            return Ok(false);
        };
        self.inner.lock().replacing = Some(previous);
        let offer = self.prepare()?;
        self.confirm(&offer.offer_id, "app-update")?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    //! The start check of a remembered install runs in a fresh probe home each time.
    //! PREDICT (before the change): the check always used `probe/saved`; on Windows a folder an elevated run made
    //! there earlier is unusable for the normal account, Vibe ends before it answers and the start fails. These tests
    //! do not compile before (no `fresh_saved_home`); after, the home is new and an unusable old folder is never used.
    use super::fresh_saved_home;

    #[test]
    fn each_check_gets_a_new_home_under_the_probe_folder() {
        let probe = std::path::Path::new("/data/vibe/probe");
        let (first, second) = (fresh_saved_home(probe), fresh_saved_home(probe));
        assert_ne!(first, second);
        assert_ne!(first, probe.join("saved"));
        assert!(first.starts_with(probe) && second.starts_with(probe));
    }

    /// An old `probe/saved` the account cannot open (made by another, elevated, account) does not fail the check.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unusable_old_probe_folder_is_not_used() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("vw-probe-fresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("vibe-app-server");
        std::fs::write(
            &program,
            "#!/bin/sh\nread line\nprintf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"serverInfo\":{\"version\":\"3.1.2\"}}}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let probe = dir.join("probe");
        let old = probe.join("saved");
        std::fs::create_dir_all(old.join("vibe-home")).unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o000)).unwrap();
        let home = fresh_saved_home(&probe);
        let result = crate::existing::probe_checked(
            std::time::Duration::from_secs(20),
            &program,
            &[],
            &home,
            &[],
        )
        .await;
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result.as_deref(), Ok("3.1.2"));
    }
}
