//! Distinguishes an undetected listener from an exited server in agent preview replies.
//! Reads the session's lease, saved launches and native process identities; called by preview ports/tools.
//! This diagnosis grants no ownership or navigation permission.

use std::sync::Arc;

use crate::core::Shared;
use vw_procmon::Snapshot;

pub(crate) fn pending_ports(
    shared: &Shared,
    session_id: &str,
    leased: &[u16],
    snapshot: &Snapshot,
) -> Vec<u16> {
    let Some(tools) = &shared.agent_tools else {
        return Vec::new();
    };
    tools
        .launches
        .all()
        .into_iter()
        .filter_map(|launch| {
            let port = launch.port?;
            (launch.session_id == session_id
                && leased.contains(&port)
                && super::agent_tools::session::alive(snapshot, launch.process)
                && !snapshot
                    .listeners
                    .iter()
                    .any(|listener| listener.port == port))
            .then_some(port)
        })
        .collect()
}

pub(crate) fn reason(port: u16) -> String {
    format!(
        "no listener detected on this port yet (port {port}); this session's process is running and \
        reserves the port. Check processes_list and its output, confirm the server uses this port, \
        then retry preview_open when it shows listening; Reload an already open preview before preview_screenshot."
    )
}

pub(crate) async fn pending(
    shared: &Arc<Shared>,
    session_id: &str,
    port: u16,
) -> Result<Option<String>, String> {
    let (shared, session_id) = (shared.clone(), session_id.to_owned());
    tokio::task::spawn_blocking(move || {
        crate::preview_ports::session_ports(&shared, &session_id)
            .map(|ports| ports.pending.contains(&port).then(|| reason(port)))
    })
    .await
    .map_err(|e| format!("the readiness check failed: {e}"))?
}
