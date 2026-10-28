//! A session id hand-off: Vibe moved a conversation to a new session id, and everything the
//! core keeps per session follows it to the id the window now shows.
//!
//! When it happens:
//! - `session/history/clear` (`/clear`) and `session/rewind` with `inplace: false` ANSWER a state with a new id
//!   (`sessions::observe` calls [`handed_off`] after loading it; the channel already moved in `channels`). The old session stays a stored session of its own.
//! - A fork is not a hand-off: the source keeps its own conversation and resources.
//!
//! What follows: the channel, the agent's tool credential (Vibe starts the new session's MCP server from the old
//! session's derived config, so it presents the old credential), the access profile and restart tracking, the
//! process monitor (registrations, re-attached processes, the port lease, the ledger), the agent's launch records,
//! the preview, the terminals, the Changes baseline and diff comments, the session copy's record and the
//! project filing. The old session's Project and File viewers close (their next use opens them for the new id).
//!
//! Depends on: `channels`, `agent_tools`, `access`, vw-procmon, `previews`, vw-terminal, `changes`, `worktrees`,
//! `projects_filing`, `pages`, `files`. Called by: `sessions::observe` (request results);
//! `new_chat` uses [`move_running`] when the first message on the New chat screen started a session.
//! Invariants:
//! - Idempotent: a second call for the same pair finds nothing left under `old` and moves nothing.
//! - Nothing is stopped or restarted: the same shells, processes and ports, owned by `new`.
//! - Each moved resource announces itself for `new` (terminal, preview, changes, comments, projects events), so a
//!   window that shows `new` re-reads it.

use std::path::PathBuf;
use std::sync::Arc;

use crate::core::Shared;
use crate::event::CoreEvent;

/// Moves every per-session resource of `old` to `new`. A no-op when the ids are equal or empty.
pub(crate) async fn handed_off(shared: &Arc<Shared>, old: &str, new: &str) {
    if old == new || old.is_empty() || new.is_empty() {
        return;
    }
    tracing::info!(old, new, "session handed off: moving its resources");
    if let Err(error) = shared.channels.rebind(old, new) {
        tracing::warn!(%error, "session hand-off could not claim ownership");
        shared.channels.close(old).await;
        return;
    }
    shared.supervisor.untrack_session(old);
    shared.supervisor.track_session(new);
    shared.access.hand_off(old, new);
    crate::model_choices::handed_off(&shared.model_choices, old, new);
    shared.plugin_refresh.hand_off(old, new);
    shared.entry_projects.hand_off(old, new);
    if let Some(tools) = &shared.agent_tools {
        tools.session_handed_off(old, new);
    }
    move_running(shared, old, new).await;
    let (hub, from, to) = (shared.clone(), old.to_owned(), new.to_owned());
    if tokio::task::spawn_blocking(move || hub.changes.hand_off(&from, &to))
        .await
        .is_ok()
    {
        shared.changes.request_refresh(new);
        shared.emit(CoreEvent::LineCommentsChanged {
            session_id: new.to_owned(),
        });
    }
    if let Ok(records) = shared.worktrees.records()
        && let Err(error) = records.hand_off(old, new)
    {
        tracing::warn!(%error, "worktrees: the copy's record did not follow the hand-off");
    }
    crate::projects_filing::handed_off(shared, old, new).await;
    crate::pages::session_ended(shared, old).await;
    crate::files::session_ended(shared, old).await;
}

/// Moves what runs for `old` to `new`: the process monitor's records and port lease, the terminals and the
/// preview with its web tabs. Shared by a Vibe hand-off and the New chat screen's first message (`new_chat`).
pub(crate) async fn move_running(shared: &Arc<Shared>, old: &str, new: &str) {
    let (monitor, from, to) = (shared.procmon.clone(), old.to_owned(), new.to_owned());
    match tokio::task::spawn_blocking(move || monitor.session_handed_off(&from, &to)).await {
        Ok(Ok(report)) => tracing::debug!(?report, "process monitor followed the hand-off"),
        Ok(Err(error)) => tracing::warn!(%error, "process ledger not rewritten after a hand-off"),
        Err(error) => tracing::warn!(%error, "process monitor hand-off task failed"),
    }
    for terminal in shared.terminals.hand_off(old, new) {
        let id = terminal.id.clone();
        shared.emit(CoreEvent::Terminal {
            event: vw_terminal::TerminalEvent::Closed {
                id,
                session_id: old.to_owned(),
            }
            .into(),
        });
        shared.emit(CoreEvent::Terminal {
            event: vw_terminal::TerminalEvent::Opened(terminal).into(),
        });
    }
    preview_handed_off(shared, old, new).await;
}

/// Revalidates retained Browser pages against the new session owner without eagerly mounting them.
async fn preview_handed_off(shared: &Arc<Shared>, old: &str, new: &str) {
    if let Some(hub) = &shared.preview {
        let new_had_preview = hub.service.view(new).is_some();
        // Keep the URLs before service transfer withdraws old port facts. The process monitor
        // already moved the lease and registrations; validate them for the new owner without
        // eagerly mounting every suspended tab.
        let local_pages: Vec<(String, String)> = hub
            .service
            .tabs(old)
            .into_iter()
            .filter_map(|tab| {
                let url = tab.preview?.url?;
                let parsed = url::Url::parse(&url).ok()?;
                matches!(
                    hub.service.policy().classify(&parsed),
                    Ok(vw_preview::Destination::Loopback { .. })
                )
                .then_some((tab.page_key, url))
            })
            .collect();
        hub.service.session_handed_off(old, new).await;
        let downloads = shared
            .names
            .cwd_of(&shared.store, new)
            .map(|cwd| PathBuf::from(cwd).join("downloads"));
        hub.service
            .rebind_handoff_downloads(new, downloads.as_deref());
        let ports = if local_pages.is_empty() {
            None
        } else {
            let (scan_shared, owner) = (shared.clone(), new.to_owned());
            match tokio::task::spawn_blocking(move || {
                crate::preview_ports::session_ports(&scan_shared, &owner)
            })
            .await
            {
                Ok(Ok(ports)) => Some(ports),
                Ok(Err(error)) => {
                    tracing::warn!(%error, "moved local preview port check failed");
                    None
                }
                Err(error) => {
                    tracing::warn!(%error, "moved local preview port check task failed");
                    None
                }
            }
        };
        for (page, url) in local_pages {
            let target = if page == old { new } else { page.as_str() };
            if page == old
                && (new_had_preview
                    || hub.service.view(old).is_some()
                    || hub.service.view(new).is_none())
            {
                continue;
            }
            if hub.service.owner_session_id(target).as_deref() != Some(new) {
                continue;
            }
            let Some(ports) = &ports else { continue };
            let Ok((checked, vw_preview::Destination::Loopback { .. })) =
                hub.service.policy().check(
                    &url,
                    |port| crate::preview_ports::port_owner(shared, ports, new, port, false),
                    true,
                )
            else {
                tracing::warn!(page_key = %page, "moved local preview no longer has this session's port");
                continue;
            };
            if !hub
                .service
                .rebind_handoff_local_ports(new, target, &checked, &ports.allowed(false))
                .await
            {
                tracing::warn!(page_key = %page, "moved local preview changed during port check");
            }
        }
    }
}
