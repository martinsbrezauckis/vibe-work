//! Debug bearer scope at command admission and value release, using the current chat mode.
//! The command and capture brokers share identity resolution; no vault permission or process is changed here.
//! Depends on: secret-run bindings and live agent access.
//! Called by: secret command admission and output capture.
use super::{Bound, Job, Shared};

pub(super) fn check_bound(shared: &Shared, session: &str, bound: &[Bound]) -> Result<(), String> {
    for binding in bound {
        crate::agent_access::check_item(shared, session, &binding.secret.id)?;
    }
    Ok(())
}

impl Job {
    pub(super) fn check_scope(&self) -> Result<(), String> {
        let shared = self.shared.upgrade().ok_or("Vibe Work closed.")?;
        check_bound(&shared, &self.session_id, &self.bound)
    }
}

/// Capture asks about saving before preparing the command. Refuse this input before that first card,
/// without changing when other input errors or ordinary vault permissions are decided.
#[cfg(feature = "agent-vault")]
pub(crate) async fn check_inputs(
    shared: &Shared,
    session: &str,
    spec: &vw_agent_tools::secretspec::RunSpec,
) -> Result<(), String> {
    if !spec.bindings.is_empty()
        && crate::agent_access::check_item(shared, session, vw_keys::DEBUG_TOKEN_ITEM).is_err()
    {
        for binding in &spec.bindings {
            crate::agent_access::check_item(shared, session, &binding.secret)?;
        }
        // Metadata resolves names; ordinary policy/read failures keep their existing capture error path.
        let Ok(secrets) = super::secrets(shared).await else {
            return Ok(());
        };
        for binding in &spec.bindings {
            if let Some(secret) = super::find_secret(&secrets, &binding.secret) {
                crate::agent_access::check_item(shared, session, &secret.id)?;
            }
        }
    }
    Ok(())
}
