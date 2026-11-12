//! Registered app-copy diagnostics for Safari; excludes the running bundle and never changes another installation.
#[cfg(any(test, all(feature = "your-safari", target_os = "macos")))]
use std::path::{Path, PathBuf};

#[cfg(any(test, all(feature = "your-safari", target_os = "macos")))]
fn other_copies(running: &Path, candidates: Vec<PathBuf>) -> Vec<String> {
    let Ok(running) = running.canonicalize() else {
        return Vec::new();
    };
    candidates
        .into_iter()
        .filter_map(|path| path.canonicalize().ok())
        .filter(|path| {
            path != &running
                && path.extension().is_some_and(|extension| extension == "app")
                && path
                    .join(super::safari_package::EXTENSION_RELATIVE_PATH)
                    .is_dir()
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(32)
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

/// Refreshes only on an explicit settings read or startup, without enumerating the filesystem.
#[cfg(all(feature = "your-safari", target_os = "macos"))]
pub(super) async fn refresh(shared: &crate::core::Shared) {
    let task = tokio::task::spawn_blocking(|| {
        let executable = std::env::current_exe().ok()?;
        let running = super::safari_package::app_at(&executable)?;
        Some(other_copies(
            &running,
            vw_platform::app_copies::registered(),
        ))
    });
    if let Ok(Ok(Some(copies))) =
        tokio::time::timeout(std::time::Duration::from_secs(2), task).await
    {
        *shared
            .chrome_control
            .safari_copies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = copies;
    }
}

#[cfg(not(all(feature = "your-safari", target_os = "macos")))]
pub(super) fn refresh(_: &crate::core::Shared) -> std::future::Ready<()> {
    std::future::ready(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_copies_excludes_running_missing_and_unrelated_bundles() {
        let root = tempfile::tempdir().unwrap();
        let running = root.path().join("Vibe Work.app");
        let other = root.path().join("Downloads/Vibe Work.app");
        let unrelated = root.path().join("Other.app");
        for app in [&running, &other] {
            std::fs::create_dir_all(
                app.join(crate::chrome_control::safari_package::EXTENSION_RELATIVE_PATH),
            )
            .unwrap();
        }
        std::fs::create_dir_all(&unrelated).unwrap();
        assert_eq!(
            other_copies(
                &running,
                vec![
                    running.clone(),
                    other.clone(),
                    unrelated,
                    root.path().join("Gone.app"),
                    other.clone()
                ]
            ),
            // The canonical spelling: `/private/var` on macOS, a verbatim `\\?\` path with `\` on Windows.
            vec![other.canonicalize().unwrap().to_string_lossy().to_string()]
        );
        assert!(other.exists());
    }

    #[cfg(unix)]
    #[test]
    fn registered_alias_of_the_running_copy_is_not_an_extra_installation() {
        let root = tempfile::tempdir().unwrap();
        let running = root.path().join("Vibe Work.app");
        std::fs::create_dir_all(
            running.join(crate::chrome_control::safari_package::EXTENSION_RELATIVE_PATH),
        )
        .unwrap();
        let alias = root.path().join("Alias.app");
        std::os::unix::fs::symlink(&running, &alias).unwrap();
        assert_eq!(other_copies(&running, vec![alias]), Vec::<String>::new());
    }
}
