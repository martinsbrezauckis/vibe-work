//! Website data for session previews and the shared public browser, per platform.
//!
//! - Linux (`WebKitGTK`) and Windows (`WebView2`): `data_directory(<storage root>/<key hex>)`: Tauri keys a `WebKitGTK`
//!   web context / a `WebView2` environment by that folder. Web pages share a browser key; Local/Lan pages keep
//!   per-session keys (the local-network page the user opened uses a folder inside its session's folder,
//!   `engine::folder_guard`, deleted with it). Wry 0.57 configures a persistent text cookie jar in that Linux directory
//!   (`webkitgtk/web_context.rs:30-48`); session-only cookies may expire when its process ends.
//! - Windows also: `data_store_identifier(<key>)`, which the patched tauri-runtime-wry (`third_party/`)
//!   applies as the `WebView2` profile name. `WEBVIEW2_USER_DATA_FOLDER` and the `UserDataFolder` policy replace
//!   every environment's folder with one shared folder. A profile per session keeps cookies apart;
//!   a shared profile can expose cookies between sessions. The engine checks the profile after the
//!   mount and refuses a preview without it.
//! - macOS 14+: `data_store_identifier(<key>)` (`WKWebsiteDataStore(forIdentifier:)`). Below 14, public browser
//!   pages use the persistent default store (also used by the trusted main window); local/LAN pages are incognito.
//!   With the feature `macos-proxy` (0.1.3, minimum macOS 14) always the identified store, never the default one:
//!   the store carries the network guard's proxy (`engine` `proxy_for`, one listener per store).
//!
//! Depends on: tauri, vw-platform (user-only folders), vw-preview (`Storage`). Called by: `engine`.
//! Invariants:
//! - The folder is created user-only before the webview is (0700 / user-only ACL).
//! - A session wipe never touches the shared browser folder (or data store).

use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, WebviewBuilder, Wry};
use vw_preview::Storage;

/// Applies the session's storage to a webview builder.
pub(crate) fn with_storage(builder: WebviewBuilder<Wry>, storage: &Storage) -> WebviewBuilder<Wry> {
    #[cfg(target_os = "macos")]
    {
        // The installed app profile keeps these
        // webviews ephemeral: WKWebsiteDataStore has no custom folder API.
        if test_profile() {
            return builder.incognito(true);
        }
        if identified_stores() {
            builder.data_store_identifier(storage.key.bytes())
        } else if storage.is_shared_web() {
            builder
        } else {
            builder.incognito(true)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Err(error) = vw_platform::ensure_private_dir(&storage.dir) {
            // The mount still gets its own folder path; WebView2 / WebKitGTK create it if needed. Logged because
            // the folder then has default permissions.
            tracing::warn!(%error, "preview storage folder could not be made user-only");
        }
        let builder = builder.data_directory(storage.dir.clone());
        #[cfg(windows)]
        let builder = builder.data_store_identifier(storage.key.bytes());
        builder
    }
}

/// macOS: whether previews get identified data stores (macOS 14+, always with the guard's proxy).
#[cfg(target_os = "macos")]
fn identified_stores() -> bool {
    cfg!(feature = "macos-proxy")
        || vw_platform::webview::macos_major_version().is_some_and(|major| major >= 14)
}

/// Deletes a session's website data after its webview closed. `foreign_profile` is the session's `WebView2`
/// profile folder when it lies outside its own folder (an overridden user data folder, see the module header).
/// Returns what is still there.
pub(crate) async fn wipe(
    app: &AppHandle<Wry>,
    storage: &Storage,
    foreign_profile: Option<PathBuf>,
) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    if identified_stores()
        && !test_profile()
        && let Err(error) = app.remove_data_store(storage.key.bytes()).await
    {
        tracing::warn!(%error, "preview data store could not be removed");
    }
    let _ = app;
    // WebView2: the browser process of the folder exits shortly after its last webview closed (Microsoft Learn,
    // "Manage user data folders": wait for it before deleting). Retry for a few seconds.
    let attempts = if cfg!(windows) { 20 } else { 1 };
    let mut left = Vec::new();
    // A foreign profile folder belongs to a browser process that serves other webviews too (the shared folder):
    // it may stay locked until the app quits; it is then listed like a busy folder.
    for dir in std::iter::once(storage.dir.clone()).chain(foreign_profile) {
        remove_with_retries(&dir, attempts).await;
        if dir.exists() {
            left.push(dir);
        }
    }
    // WORKAROUND(tauri-runtime-wry 2.12.1 lib.rs:2391-2414, tauri#14626): on Linux the web context of a closed
    // webview stays alive until the app quits and its network process may still write (cookies). The folder is
    // listed for one more wipe at the next start. Exit: Tauri releases web contexts on Linux.
    if cfg!(target_os = "linux") && !left.contains(&storage.dir) {
        left.push(storage.dir.clone());
    }
    left
}

/// Removes `dir`, retrying `attempts` times 300 ms apart while it is in use.
async fn remove_with_retries(dir: &PathBuf, attempts: u32) {
    for attempt in 0..attempts {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) if attempt + 1 < attempts => {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            Err(error) => tracing::info!(%error, "preview storage still in use"),
        }
    }
}

#[cfg(test)]
#[path = "storage_readonly_tests.rs"]
mod readonly_tests;

#[cfg(any(test, target_os = "macos"))]
fn test_profile() -> bool {
    vw_platform::live_profile::root().ok().flatten().is_some()
}

#[cfg(test)]
mod profile_tests {
    #[test]
    fn unrelated_or_invalid_profile_does_not_change_storage() {
        if std::env::var_os("VW_STORAGE_PROFILE_CHILD").is_some() {
            assert!(!super::test_profile());
            return;
        }
        for value in ["", "unrelated", "/does-not-exist/vw-profile"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "storage::profile_tests::unrelated_or_invalid_profile_does_not_change_storage",
                ])
                .env("VW_STORAGE_PROFILE_CHILD", "1")
                .env("VW_LIVE_SETUP_PROFILE", value)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}
