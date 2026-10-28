//! Rechecks user-owned installs before launch and replaces an older app-owned runtime after an app update.
//! Called by the core before launch and after attaching the install progress/ready hooks.
use crate::{InstallError, InstallPhase, InstallSource, VibeInstaller, layout::Layout};

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
        let version = crate::existing::probe(
            &choice.path,
            &layout.root.join("probe/saved"),
            &self.inner.config.parent_env,
        )
        .await;
        let supported = version.as_deref() == Some(&self.inner.pinned);
        let warning = Some(version.as_deref().map_or_else(
            || {
                format!(
                    "This install did not answer; Vibe Work needs Vibe {}.",
                    self.inner.pinned
                )
            },
            |found| crate::existing::version_warning(found, &self.inner.pinned),
        ));
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
