//! The URL rules of mounted previews (including macOS content rules) and the ports they follow: every mounted Local
//! preview gets new rules when the set of ports any session's preview may open changes (an open adds ports; a stopped
//! server, an ended lease, a closed preview or an ended session removes them), before the page that needs them loads.
//!
//! Depends on: `service` (the registry, the engine, `bounded`), `state` (`rule_page`), vw-netguard (`rules`). Called
//! by: `service` (open, close, session end), the core (`retain_allowed_ports`, its periodic port check).
//! Invariants:
//! - Only an open or a checked session handoff adds a port to a session's own set; the re-check only withdraws from it
//!   ([`PreviewService::retain_allowed_ports`]). Other sessions' servers come from the re-check
//!   ([`PreviewService::set_server_ports`]), as the egress guard reads them from the process monitor.
//! - Refreshes run one at a time, each reading the newest state when it starts (an older, wider list never lands
//!   after a newer one); the macOS engine also drops a compile that finishes after a newer one started
//!   (`vw_netguard::rules::RuleGenerations`).
//! - An update is sent only when a mount's rules really change (or a failed one is pending), and the caller waits
//!   until it applied (bounded). The recorded rules are always what the engine confirmed; a narrowing update the
//!   engine could not apply closes that preview.

use vw_netguard::rules::RulePage;

use crate::engine::MountId;
use crate::service::PreviewService;

impl PreviewService {
    /// Pages with port rights or an active stopped server awaiting Reload. Retained URLs of ended sessions do not
    /// keep the monitor running.
    #[must_use]
    pub fn sessions_with_ports(&self) -> Vec<String> {
        self.inner
            .lock()
            .entries
            .iter()
            .filter(|(_, e)| !e.allowed.is_empty() || e.server_status.is_some())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Sets the ports every session's servers listen on now (the core's re-check); every mounted Local preview's
    /// rules follow when the set changed.
    pub async fn set_server_ports(&self, mut ports: Vec<u16>) {
        ports.sort_unstable();
        ports.dedup();
        let changed = {
            let mut state = self.inner.lock();
            let changed = state.server_ports != ports;
            state.server_ports = ports;
            changed
        };
        if changed {
            self.refresh_rules().await;
        }
    }

    /// Keeps only the ports of `session_id`'s preview that are still its own (`current`: listeners attributed to the
    /// session and its leased ports, as the core sees them now); a stopped server or an ended lease leaves the
    /// set, and the URL rules of every mounted Local preview follow at once. Never adds a port
    /// (only an open does). Returns true when a port was withdrawn.
    pub async fn retain_allowed_ports(&self, session_id: &str, current: &[u16]) -> bool {
        let shrunk = {
            let mut state = self.inner.lock();
            match state.entries.get_mut(session_id) {
                Some(entry) => {
                    let before = entry.allowed.len();
                    entry.allowed.retain(|p| current.contains(p));
                    entry.allowed.len() != before
                }
                None => false,
            }
        };
        if shrunk {
            tracing::info!(session = %session_id, "preview ports withdrawn (server stopped or lease ended)");
            self.refresh_rules().await;
            self.inner.changed(session_id);
        }
        shrunk
    }

    /// Gives every mounted Local preview the URL rules for the current ports when they changed (an open
    /// added ports, a server stopped, a lease ended, a preview or session closed), and waits until the engine applied
    /// them (bounded). The applied rules are recorded only after the engine confirms them; a
    /// failed update stays pending and every later refresh retries it; a failed update that would withdraw a port the
    /// confirmed rules allow closes that preview (suspended: it opens again with fresh rules), so a withdrawn port
    /// never stays reachable. Vibe Work's own ports unknown: Local rules fail closed (`rule_page`).
    pub(crate) async fn refresh_rules(&self) {
        // Serialized to prevent out-of-order completion: the next refresh reads state after this one ends.
        let _one_at_a_time = self.inner.rules_lock.lock().await;
        let internal = self.inner.internal_ports();
        if internal.is_none() {
            tracing::warn!(
                "Vibe Work's own ports are unknown: preview URL rules allow only each session's own ports"
            );
        }
        let updates: Vec<(MountId, RulePage, bool)> = {
            let mut state = self.inner.lock();
            let local: Vec<String> = state
                .entries
                .iter()
                .filter(|(_, e)| {
                    e.ready_mount().is_some_and(|m| {
                        m.class == crate::policy::PageClass::Local
                            && (m.rules.is_some() || m.rules_pending)
                    })
                })
                .map(|(id, _)| id.clone())
                .collect();
            let mut updates = Vec::new();
            for session in local {
                let page = state.rule_page(
                    &session,
                    crate::policy::PageClass::Local,
                    internal.as_deref(),
                );
                if let Some(mount) = state.entry(&session).mount.as_ref()
                    && (mount.rules_pending || mount.rules.as_ref() != Some(&page))
                {
                    let narrows = narrows(mount.rules.as_ref(), &page);
                    updates.push((
                        MountId {
                            session_id: session.clone(),
                            label: mount.label.clone(),
                        },
                        page,
                        narrows,
                    ));
                }
            }
            updates
        };
        for (id, page, narrows) in updates {
            let result = self
                .bounded("rules", self.engine.update_rules(&id, page.clone()))
                .await;
            let close = {
                let mut state = self.inner.lock();
                let mount = state
                    .entries
                    .get_mut(&id.session_id)
                    .and_then(|e| e.mount.as_mut())
                    .filter(|m| m.label == id.label);
                match (mount, &result) {
                    (Some(mount), Ok(())) => {
                        mount.rules = Some(page);
                        mount.rules_pending = false;
                        false
                    }
                    (Some(mount), Err(error)) => {
                        tracing::warn!(%error, session = %id.session_id, narrows, "preview URL rules not updated");
                        mount.rules_pending = true;
                        narrows
                    }
                    // Closed or replaced meanwhile: its successor got its own rules at mount.
                    (None, _) => false,
                }
            };
            if close {
                tracing::warn!(session = %id.session_id, "preview closed until its URL rules apply");
                if let Some(entry) = self.inner.lock().entries.get_mut(&id.session_id)
                    && let Some(url) = entry.url.clone()
                {
                    entry.note_refused(
                        url.as_str(),
                        "the preview was closed while its address rules changed; open it again",
                    );
                }
                let _ = self.unmount(&id.session_id).await;
                self.inner.changed(&id.session_id);
            }
        }
    }
}

/// True when `new` would take away a right the confirmed rules `old` give (a port). Nothing confirmed (`None`): the
/// mount runs without URL rules already (reported by the missing layer), so closing it would protect nothing.
fn narrows(old: Option<&RulePage>, new: &RulePage) -> bool {
    match (old, new) {
        (Some(RulePage::Local { ports: before }), RulePage::Local { ports: after }) => {
            before.iter().any(|p| !after.contains(p))
        }
        (Some(old), new) => old != new,
        (None, _) => false,
    }
}
