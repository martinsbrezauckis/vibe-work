//! Forwards key-service status and remote cleanup notices to existing core event subscribers.
//! Depends on the shared core and key service; startup installs the forwarder, and it exits with the core.

use crate::core::Shared;
use crate::event::CoreEvent;
use std::sync::Weak;

/// Re-emits the key service's events as core events until the core goes away.
pub(crate) async fn forward(
    shared: Weak<Shared>,
    mut events: tokio::sync::broadcast::Receiver<vw_keys::KeyEvent>,
) {
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                vw_keys::KeyEvent::StatusChanged
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let Some(shared) = shared.upgrade() else {
            return;
        };
        if event == vw_keys::KeyEvent::KeyChanged {
            crate::recovery::reopen_after_key_change(&shared).await;
        }
        shared.emit(match event {
            vw_keys::KeyEvent::KeyNeeded(need) => CoreEvent::KeyNeeded {
                reason: match need {
                    vw_keyproxy::KeyNeed::Unlock => "unlock",
                    vw_keyproxy::KeyNeed::Setup => "setup",
                }
                .to_owned(),
            },
            vw_keys::KeyEvent::RateLimits(limits) => CoreEvent::RateLimits {
                limits: limits.map(Into::into),
            },
            vw_keys::KeyEvent::RemoteFileCleanupFailed(message) => CoreEvent::Alert {
                session_id: None,
                kind: crate::event::AlertKind::RemoteFileCleanupFailed,
                message,
            },
            vw_keys::KeyEvent::NetworkSettingsChecked(false) => {
                crate::network::rejection_notice(shared.config.keys.as_deref())
            }
            // Saved settings that waited for the vault are in use: later subscribers get no notice for them.
            vw_keys::KeyEvent::NetworkSettingsChecked(true) => {
                shared
                    .network_notice
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                CoreEvent::KeyStatusChanged {}
            }
            vw_keys::KeyEvent::StatusChanged | vw_keys::KeyEvent::KeyChanged => {
                let _ = shared
                    .secret_scan
                    .send(crate::secret_scan::Job::VaultChanged);
                if shared.secret_scan.view().status == crate::SecretScanStatus::Locked {
                    shared
                        .secret_scan
                        .pending
                        .store(true, std::sync::atomic::Ordering::Release);
                }
                CoreEvent::KeyStatusChanged {}
            }
        });
    }
}
