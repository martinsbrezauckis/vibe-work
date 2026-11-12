//! Embedded Safari assets, found only inside the running Vibe Work app bundle.
//! The signed build supplies identities; agent arguments and environment cannot select another containing app.
use std::path::{Path, PathBuf};

pub(super) const EXTENSION_RELATIVE_PATH: &str = "Contents/PlugIns/Your Safari Extension.appex";

pub(super) fn app_at(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then(|| app.to_path_buf())
}

fn app_path() -> Option<PathBuf> {
    app_at(&std::env::current_exe().ok()?)
}

fn assets_present(app: &Path) -> bool {
    app.join(EXTENSION_RELATIVE_PATH).is_dir() && app.join("Contents/MacOS/SafariSetup").is_file()
}

pub(super) fn installed() -> bool {
    option_env!("VW_SAFARI_APP_GROUP").is_some()
        && option_env!("VW_SAFARI_EXTENSION_ID").is_some()
        && app_path().is_some_and(|app| assets_present(&app))
}

#[cfg(all(feature = "your-safari", target_os = "macos"))]
pub(super) async fn configure(
    command: &mut tokio::process::Command,
    shared: &std::sync::Weak<crate::core::Shared>,
) {
    if let (Some(group), Some(extension)) = (
        option_env!("VW_SAFARI_APP_GROUP"),
        option_env!("VW_SAFARI_EXTENSION_ID"),
    ) {
        // Resolve through entitled code before the relay starts; a Safari failure leaves Chrome usable.
        let prepared = prepare().await.is_ok();
        if let Some(shared) = shared.upgrade() {
            super::safari::connection(
                &shared,
                if prepared {
                    "preparing"
                } else {
                    "containerUnavailable"
                },
            );
        }
        if prepared {
            command.env("CHROME_CONTROL_SAFARI_GROUP", group);
            command.env("CHROME_CONTROL_SAFARI_EXTENSION_ID", extension);
        }
    }
}

#[cfg(all(feature = "your-safari", target_os = "macos"))]
async fn prepare() -> Result<(), ()> {
    let app = app_path().ok_or(())?;
    prepare_at(
        &app.join("Contents/MacOS/SafariSetup"),
        option_env!("VW_SAFARI_APP_GROUP").ok_or(())?,
    )
    .await
}

#[cfg(any(all(feature = "your-safari", target_os = "macos"), all(test, unix)))]
async fn prepare_at(setup: &Path, group: &str) -> Result<(), ()> {
    use std::{process::Stdio, time::Duration};
    let status = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(setup)
            .args(["--prepare-group", group])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())?;
    if status.success() { Ok(()) } else { Err(()) }
}

#[cfg(target_os = "macos")]
pub(super) async fn registration(remove: bool) -> Result<(), String> {
    let app = app_path().ok_or("Safari is unavailable outside the Vibe Work app bundle.")?;
    super::safari_registration::change_at(Path::new("/usr/bin/pluginkit"), &app, remove).await
}

#[cfg(target_os = "macos")]
pub(super) fn open(shared: &std::sync::Arc<crate::core::Shared>) -> Result<(), String> {
    // Only the containing app may check, configure or message its Safari extension (see
    // `vw_platform::safari_extension`), so this app process takes the step; the bundled setup tool only prepares
    // the App Group. The person just clicked this app, so it is the active one and brings Safari forward first.
    app_path().ok_or("Safari is unavailable outside the Vibe Work app bundle.")?;
    let extension =
        option_env!("VW_SAFARI_EXTENSION_ID").ok_or("Safari extension identity is missing.")?;
    let launching = !vw_platform::browser::safari_running();
    let seen = shared
        .chrome_control
        .owner
        .safari_seen
        .load(std::sync::atomic::Ordering::SeqCst);
    let forwarded = vw_platform::browser::bring_safari_forward(|allowed| {
        tracing::info!(allowed, "Safari brought forward after its launch");
    });
    if !forwarded {
        tracing::warn!("Safari was not found to bring forward");
    }
    let shared = std::sync::Arc::downgrade(shared);
    vw_platform::safari_extension::open_setup(
        extension,
        move || {
            if launching {
                wait_for_extension(&shared, seen);
            }
        },
        |result| match result {
            Ok(outcome) => tracing::info!(?outcome, "Safari setup step done"),
            Err(reason) => tracing::warn!(%reason, "Safari setup step failed"),
        },
    )
    .map_err(|_| "Safari Settings could not be opened.".to_owned())
}

/// A Safari launched for the pairing notice has no extension port yet, and Apple drops a notice sent before one
/// opens while still reporting success. The extension's first message after launch reaches the helper, which
/// reports a Safari pairing state (`safari::observe` counts these), so the notice waits for that, bounded.
#[cfg(target_os = "macos")]
fn wait_for_extension(shared: &std::sync::Weak<crate::core::Shared>, seen: u64) {
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(20) {
        let Some(live) = shared.upgrade() else {
            return;
        };
        let now = live
            .chrome_control
            .owner
            .safari_seen
            .load(std::sync::atomic::Ordering::SeqCst);
        drop(live);
        if now != seen {
            tracing::info!(
                waited_ms = started.elapsed().as_millis(),
                "Safari extension connected after launch"
            );
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    tracing::warn!(
        "Safari extension did not connect after launch; sending the pairing notice anyway"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn safari_container_preparation_waits_for_the_embedded_utility_and_refuses_failure() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let setup = root.path().join("SafariSetup");
        std::fs::write(
            &setup,
            "#!/bin/sh\n[ \"$1\" = --prepare-group ] && [ \"$2\" = ABCDE12345.app.test ]\n",
        )
        .unwrap();
        std::fs::set_permissions(&setup, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(prepare_at(&setup, "ABCDE12345.app.test").await.is_ok());
        assert!(prepare_at(&setup, "wrong-group").await.is_err());
        assert!(
            prepare_at(&root.path().join("missing"), "ABCDE12345.app.test")
                .await
                .is_err()
        );
    }

    #[test]
    fn embedded_assets_follow_the_running_app_when_moved() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("Moved Vibe Work.app");
        assert_eq!(
            app_at(&app.join("Contents/MacOS/vibe-work")),
            Some(app.clone())
        );
        assert_eq!(app_at(&app.join("Resources/other")), None);
        assert_eq!(app_at(Path::new("/tmp/vibe-work")), None);
        std::fs::create_dir_all(app.join(EXTENSION_RELATIVE_PATH)).unwrap();
        assert!(!assets_present(&app));
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/MacOS/SafariSetup"), b"fixture").unwrap();
        assert!(assets_present(&app));
        std::fs::remove_dir(app.join(EXTENSION_RELATIVE_PATH)).unwrap();
        assert!(!assets_present(&app));
    }
}
