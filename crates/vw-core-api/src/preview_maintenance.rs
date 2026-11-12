//! Preview maintenance: release idle browsers, report stopped servers and withdraw expired port rights.
//! Depends on preview changes and process attribution; started and aborted by the preview hub.
//! Sleeps without a timer when there are no admitted browsers, port rights or stopped-server notices.

use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use crate::core::Shared;

pub(super) fn start(shared: &Arc<Shared>) -> JoinHandle<()> {
    let weak = Arc::downgrade(shared);
    // Subscribe before checking work so an open between the check and recv cannot be missed.
    let mut changes = shared.preview.as_ref().map(|hub| hub.service.changes());
    tokio::spawn(async move {
        let Some(changes) = changes.as_mut() else {
            return;
        };
        loop {
            let Some(active) = weak.upgrade().and_then(|shared| {
                shared.preview.as_ref().map(|hub| {
                    hub.service.has_active_browsers()
                        || !hub.service.sessions_with_ports().is_empty()
                })
            }) else {
                return;
            };
            if !active {
                if matches!(changes.recv().await, Err(RecvError::Closed)) {
                    return;
                }
                continue;
            }
            tokio::time::sleep(super::PORTS_RECHECK).await;
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let Some(hub) = shared.preview.clone() else {
                return;
            };
            #[cfg(test)]
            hub.maintenance_counts
                .rounds
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Err(error) = hub.service.reap_idle_browsers().await {
                tracing::debug!(%error, "idle browser release failed");
            }
            let sessions = hub.service.sessions_with_ports();
            if sessions.is_empty() {
                continue;
            }
            let mut owners: Vec<String> = sessions
                .iter()
                .filter_map(|page| hub.service.owner_session_id(page))
                .collect();
            owners.sort_unstable();
            owners.dedup();
            let scan = {
                #[cfg(test)]
                hub.maintenance_counts
                    .scans
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let owned = shared.clone();
                tokio::task::spawn_blocking(move || {
                    crate::preview_ports::live_ports(&owned, &owners)
                })
                .await
            };
            match scan {
                Ok(Ok(live)) => {
                    for page in &sessions {
                        if let Some(owner) = hub.service.owner_session_id(page) {
                            if let Some(port) = hub.service.local_server_port(page) {
                                let listening = live
                                    .listeners
                                    .get(&owner)
                                    .is_some_and(|ports| ports.contains(&port));
                                let _ = hub
                                    .service
                                    .set_local_server_listening(page, port, listening);
                            }
                            let current = live.sessions.get(&owner).cloned().unwrap_or_default();
                            let _ = hub.service.retain_allowed_ports(page, &current).await;
                        }
                    }
                }
                // A failed scan cannot prove that a port should be withdrawn.
                Ok(Err(error)) => tracing::debug!(%error, "preview port re-check skipped"),
                Err(error) => tracing::debug!(%error, "preview port re-check failed"),
            }
        }
    })
}
