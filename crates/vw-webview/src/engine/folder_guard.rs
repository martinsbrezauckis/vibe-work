//! Which guard listener a mount's traffic goes through on Linux and Windows: one per data folder.
//!
//! Both engines keep the proxy per data folder, not per webview: `WebView2` takes it from the browser arguments, and
//! every environment on one user data folder shares that folder's single browser process and must use the same
//! arguments; `WebKitGTK` keeps it on the web context, which Tauri keeps one per folder (the last webview's setting
//! carries every page of the folder). So the guard gets one listener per folder (`store_key`, a store listener: it
//! grants only what every mount on that folder may reach), never one listener per mount or for every preview: a
//! request carries the live mounts' shared scope. Browser mounts allow local addresses. Storage remains split
//! between the shared public-web folder, each session's own-server folder and per-host local-network folders.
//! Such a separate folder runs in its own browser process (Windows) or web context (Linux) while open.
//!
//! Depends on: vw-preview (`MountSpec`, `StorageKey`), vw-netguard (`rules::RulePage`), vw-platform (user-only
//! folders). Called by: the engine (`proxy_for`, `mount_inner`, `wipe`) and the account anchor.

use std::path::Path;

/// The guard's store key of the user data folder `dir` (one listener per folder).
pub(super) fn store_key(dir: &Path) -> String {
    format!("folder:{}", dir.display())
}

/// The prefix of the store keys of the folders inside `dir` (a session's local-network page folders).
pub(super) fn inner_store_prefix(dir: &Path) -> String {
    format!("{}{}", store_key(dir), std::path::MAIN_SEPARATOR)
}

/// Linux, Windows: the mount spec with the data folder its pages use. The local-network page the user opened gets
/// `<session folder>/lan-<key of the session and host>` (kept across remounts, so its sign-ins stay like any page of
/// the session; deleted with the session's folder); every other spec is unchanged. macOS: unchanged (the page gets
/// a data store of its own there, `mac_guard`).
pub(super) fn with_own_folder(mut spec: vw_preview::MountSpec) -> vw_preview::MountSpec {
    if cfg!(any(windows, target_os = "linux"))
        && let Some(vw_netguard::rules::RulePage::Lan { host }) = &spec.rules
    {
        let name = vw_preview::StorageKey::of(&format!(
            "{}\nlocal-network page\n{}",
            spec.storage.key.hex(),
            host.trim_end_matches('.').to_ascii_lowercase()
        ));
        // The session's folder holds this one: user-only first, as `storage` makes every folder it gives a webview.
        if let Err(error) = vw_platform::ensure_private_dir(&spec.storage.dir) {
            tracing::warn!(%error, "preview storage folder could not be made user-only");
        }
        spec.storage.dir = spec.storage.dir.join(format!("lan-{}", name.hex()));
    }
    spec
}
