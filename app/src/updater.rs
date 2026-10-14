//! Update wiring: registers `tauri-plugin-updater` only for a build whose config carries an
//! accepted update channel (`vw_host::updates::channel`: pubkey, https or test-only loopback endpoints,
//! `requireSignedVersion`), and hands the plugin to the host's update flow (`vw_host::update_flow`), which owns
//! the policy: notify, download, then ask. This file is the Tauri side only:
//! - [`PluginBackend`]: check / download / install with the plugin's `Update`;
//! - [`AppHooks`]: the OS notification and the relaunch after an install (core shut down first);
//! - [`UpdatesHandle`]: the manager as Tauri state for the commands;
//! - [`VibeSlot`]: the run's Vibe installer, so the same check also reads the signed supported-Vibe
//!   manifest (`vibe-support.json` next to `latest.json`, verified with the same `pubkey`).
//!
//! Depends on: tauri, tauri-plugin-updater, vw-host (`update_flow`, `updates`), vw-platform (notification).
//! Called by: `lib.rs` (plugin registration, setup), `commands.rs` (the `update_*` commands).
//! Invariants: a build without an accepted channel never registers the plugin, creates no manager and makes no
//! update request; development builds never check; the plugin's `install` is reachable only through the flow's
//! consent-checked `install` (this file has no other caller of it).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};
use vw_host::HostHandle;
use vw_host::update_flow::{
    BoxFuture, InstallMode, Offer, Progress, UpdateBackend, UpdateConfig, UpdateHooks,
    UpdateManager, UpdateNotice,
};
use vw_host::updates::{UpdatesOff, channel};
use vw_platform::{NotificationRequest, Notifier};

/// How long after start the check runs (the window and Vibe come first).
const CHECK_DELAY: Duration = Duration::from_secs(15);

/// The run's Vibe installer (set by the startup when it builds the product launch), for the Vibe update check that
/// runs with every app update check. Empty in development builds (they start their own server).
#[derive(Default)]
pub struct VibeSlot(pub OnceLock<vw_vibe_install::VibeInstaller>);

/// The update manager as Tauri state; empty in a build without an update channel.
#[derive(Default)]
pub struct UpdatesHandle(OnceLock<UpdateManager>);

impl UpdatesHandle {
    /// The manager, when this build has an update channel and the app has started.
    pub fn get(&self) -> Option<&UpdateManager> {
        self.0.get()
    }
}

/// The plugin, when this build's config has an accepted update channel; otherwise `None` and the reason
/// is logged.
pub fn plugin_if_configured<R: Runtime>(
    config: &tauri::Config,
) -> Option<tauri::plugin::TauriPlugin<R, tauri_plugin_updater::Config>> {
    match channel(config.plugins.0.get("updater")) {
        Ok(channel) => {
            tracing::info!(endpoints = channel.endpoints.len(), "update channel on");
            Some(tauri_plugin_updater::Builder::new().build())
        }
        Err(UpdatesOff::NotConfigured) => {
            tracing::info!("update checks off: no update channel in this build");
            None
        }
        Err(error) => {
            tracing::warn!(%error, "update checks off");
            None
        }
    }
}

/// Creates the manager for this run, stores it in `handle` and, in a user build, schedules the one check after
/// start. Called from `setup` when [`plugin_if_configured`] returned a plugin.
pub fn start<R: Runtime>(
    app: &AppHandle<R>,
    handle: &UpdatesHandle,
    data_dir: PathBuf,
    dev_build: bool,
) {
    let Some(pubkey) = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|c| c.get("pubkey"))
        .and_then(|k| k.as_str())
        .map(str::to_owned)
    else {
        return;
    };
    let bundle = tauri::utils::platform::bundle_type().map(|b| b.to_string());
    let mode = InstallMode::for_bundle(bundle.as_deref());
    tracing::info!(?bundle, ?mode, "update install mode");
    let manager = UpdateManager::new(
        UpdateConfig {
            current: app.package_info().version.to_string(),
            pubkey,
            mode,
            data_dir,
        },
        Arc::new(PluginBackend {
            app: app.clone(),
            cached: Arc::new(Mutex::new(None)),
        }),
        Arc::new(AppHooks { app: app.clone() }),
    );
    attach_vibe_check(app, &manager);
    if !dev_build {
        manager.spawn_check(&tauri::async_runtime::handle().inner().clone(), CHECK_DELAY);
    }
    let _ = handle.0.set(manager);
}

/// Every update check also checks the signed supported-Vibe manifest next to `latest.json` (same channel,
/// same `pubkey`); no other network. Nothing is attached when the channel has no fixed folder for it.
fn attach_vibe_check<R: Runtime>(app: &AppHandle<R>, manager: &UpdateManager) {
    let config = app.config().plugins.0.get("updater").cloned();
    let Ok(channel) = channel(config.as_ref()) else {
        return;
    };
    let pubkey = config
        .as_ref()
        .and_then(|c| c.get("pubkey"))
        .and_then(|k| k.as_str())
        .unwrap_or_default()
        .to_owned();
    let Some(manifest_url) = channel.sibling(vw_vibe_install::support::MANIFEST_FILE) else {
        tracing::info!("no fixed channel folder: the Vibe update check is off");
        return;
    };
    let support = vw_vibe_install::SupportConfig {
        manifest_url: manifest_url.to_string(),
        pubkey,
        app_version: app.package_info().version.to_string(),
        target: vw_vibe_install::this_target(),
    };
    let app = app.clone();
    manager.set_companion_check(Arc::new(move || {
        let app = app.clone();
        let support = support.clone();
        Box::pin(async move {
            let installer = app.try_state::<VibeSlot>().and_then(|s| s.0.get().cloned());
            if let Some(installer) = installer
                && let Err(error) = installer.check_support(&support).await
            {
                tracing::info!(%error, "Vibe update check: no offer");
            }
        })
    }));
}

/// The updater plugin as the flow's backend. Keeps the last `Update` the plugin returned: the flow downloads and
/// installs exactly that announcement (same version and signature), or checks again.
struct PluginBackend<R: Runtime> {
    app: AppHandle<R>,
    cached: Arc<Mutex<Option<Update>>>,
}

fn matches_offer(update: &Update, offer: &Offer) -> bool {
    update.version == offer.version && update.signature == offer.signature
}

fn offer_of(update: &Update) -> Offer {
    Offer {
        version: update.version.clone(),
        notes: update.body.clone(),
        signature: update.signature.clone(),
        // No link from the manifest: the "Open release page" button goes to the official repository's tag for this
        // version (`vw_host::update_flow::release_page_for`).
    }
}

/// The updater, with the core shut down before a Windows installer takes the process (the plugin runs
/// `on_before_exit` only there; on Linux and macOS the flow's `restart` hook does the shutdown).
fn build_updater<R: Runtime>(app: &AppHandle<R>) -> Result<tauri_plugin_updater::Updater, String> {
    let exit_app = app.clone();
    app.updater_builder()
        .on_before_exit(move || {
            shut_down_core(&exit_app);
            exit_app.cleanup_before_exit();
        })
        .build()
        .map_err(|e| e.to_string())
}

fn shut_down_core<R: Runtime>(app: &AppHandle<R>) {
    if let Some(host) = app.state::<HostHandle>().try_get() {
        tauri::async_runtime::block_on(host.shutdown());
    }
}

impl<R: Runtime> UpdateBackend for PluginBackend<R> {
    fn check(&self) -> BoxFuture<Result<Option<Offer>, String>> {
        let app = self.app.clone();
        let cached = self.cached.clone();
        Box::pin(async move {
            let found = build_updater(&app)?
                .check()
                .await
                .map_err(|e| e.to_string())?;
            let offer = found.as_ref().map(offer_of);
            *cached.lock().unwrap_or_else(PoisonError::into_inner) = found;
            Ok(offer)
        })
    }

    fn download(&self, offer: Offer, progress: Progress) -> BoxFuture<Result<Vec<u8>, String>> {
        let cached = self.cached.clone();
        Box::pin(async move {
            let update = cached
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .filter(|update| matches_offer(update, &offer))
                .ok_or("the update server's announcement changed; check again")?;
            let received = Mutex::new(0_u64);
            update
                .download(
                    |chunk, total| {
                        let mut so_far = received.lock().unwrap_or_else(PoisonError::into_inner);
                        *so_far += chunk as u64;
                        progress(*so_far, total);
                    },
                    || {},
                )
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn install(&self, offer: Offer, package: Vec<u8>) -> BoxFuture<Result<(), String>> {
        let app = self.app.clone();
        let cached = self.cached.clone();
        Box::pin(async move {
            // After a restart the plugin's `Update` is gone: ask the server again and require the same
            // announcement (the package on disk was verified for exactly this version and signature).
            let held = cached
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .filter(|update| matches_offer(update, &offer));
            let update = match held {
                Some(update) => update,
                None => build_updater(&app)?
                    .check()
                    .await
                    .map_err(|e| e.to_string())?
                    .filter(|update| matches_offer(update, &offer))
                    .ok_or("the update server no longer offers this version; check again")?,
            };
            // The installer may exit the process (Windows) or block on file work: not on the async workers.
            tokio::task::spawn_blocking(move || update.install(&package).map_err(|e| e.to_string()))
                .await
                .map_err(|e| e.to_string())?
        })
    }
}

/// OS notification and relaunch.
struct AppHooks<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> UpdateHooks for AppHooks<R> {
    fn notify(&self, notice: UpdateNotice) {
        let (title, body) = notice.text();
        let notifier =
            vw_platform::DesktopNotifier::new(self.app.config().identifier.clone(), true);
        let app = self.app.clone();
        if let Err(error) = notifier.show(
            NotificationRequest { title, body },
            Box::new(move || {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
                if let Some(host) = app
                    .try_state::<HostHandle>()
                    .and_then(|handle| handle.try_get())
                {
                    host.open_updates();
                }
            }),
        ) {
            tracing::info!(%error, "update notification not shown");
        }
    }

    fn restart(&self) -> BoxFuture<()> {
        let app = self.app.clone();
        Box::pin(async move {
            if let Some(host) = app.state::<HostHandle>().try_get() {
                host.shutdown().await;
            }
            app.restart()
        })
    }
}

/// The release manifest format: what `make-latest-json` writes must parse with the updater plugin's own
/// `RemoteRelease` and resolve every shipped target, so a release can never publish a manifest the app cannot read.
#[cfg(test)]
mod manifest_format {
    use tauri_plugin_updater::RemoteRelease;

    #[test]
    fn the_release_manifest_parses_with_the_plugin() {
        let base = make_latest_json::release_base("1.2.3");
        let entries: Vec<(String, String, String)> = make_latest_json::PLATFORMS
            .iter()
            .map(|(key, _)| {
                (
                    (*key).to_owned(),
                    format!("{base}asset-{key}"),
                    format!("sig-{key}"),
                )
            })
            .collect();
        let borrowed: Vec<(&str, &str, &str)> = entries
            .iter()
            .map(|(k, u, s)| (k.as_str(), u.as_str(), s.as_str()))
            .collect();
        let text = make_latest_json::render("1.2.3", "Notes.", "2026-10-12T09:00:00Z", &borrowed);
        let release: RemoteRelease = serde_json::from_str(&text).unwrap();
        assert_eq!(release.version.to_string(), "1.2.3");
        assert_eq!(release.notes.as_deref(), Some("Notes."));
        assert_eq!(release.pub_date.unwrap().unix_timestamp(), 1_791_795_600);
        // The plugin looks up `{os}-{arch}-{installer}`, then `{os}-{arch}`: the plain keys are what it finds.
        for (key, url, signature) in &entries {
            assert_eq!(release.download_url(key).unwrap().as_str(), url);
            assert_eq!(release.signature(key).unwrap(), signature);
        }
        // The running build's own target is one of them (x86_64 Windows/Linux, Apple-silicon macOS).
        if let Some(target) = tauri_plugin_updater::target() {
            let shipped = make_latest_json::PLATFORMS
                .iter()
                .any(|(key, _)| *key == target);
            let expected = cfg!(any(
                all(target_os = "windows", target_arch = "x86_64"),
                all(target_os = "linux", target_arch = "x86_64"),
                all(target_os = "macos", target_arch = "aarch64")
            ));
            assert_eq!(shipped, expected, "target {target}");
        }
    }
}
