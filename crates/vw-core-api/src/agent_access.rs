//! The window's agent access switch for project vault data. The switch lives only for this core run and leaves
//! the cryptographic vault and model key proxy untouched. It starts unlocked on every core restart. Agent tools,
//! browser fill and the MCP resolver call it.
//!
//! Depends on: `core::Shared`. Called by: `secrets`, `agent_tools`, `browser_fill`, `browser_secret`, `mcp_keys`.

use std::sync::atomic::Ordering;

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

/// The exact response shown for every paused agent vault path.
pub(crate) fn locked_message() -> &'static str {
    vw_vocabulary::text("agentPausedRefusal").unwrap_or("agentPausedRefusal")
}

/// True when the user has paused agent access.
pub(crate) fn locked(shared: &Shared) -> bool {
    shared.agent_access_locked.load(Ordering::SeqCst)
}

/// Refuses access while paused.
pub(crate) fn check(shared: &Shared) -> Result<(), String> {
    if locked(shared) {
        Err(locked_message().to_owned())
    } else {
        Ok(())
    }
}

/// [`check`] at the start of an agent's vault use (`kind` is the ask policy kind, `session` the agent's session):
/// the pause is the user's explicit "no agent access" switch, so it refuses without a prompt, and records the
/// refusal (an `Outcome::Never` record and a Security bell entry) so it is visible.
pub(crate) fn check_for(shared: &Shared, kind: &str, session: &str) -> Result<(), String> {
    check(shared).inspect_err(|_| {
        shared.asks.record_never(kind, Some(session.to_owned()));
    })
}

/// Sets the pause after a user-only command has passed `vault_scope`.
pub(crate) fn set(shared: &Shared, locked: bool) {
    shared.agent_access_locked.store(locked, Ordering::SeqCst);
}

/// Applies the user's switch; unlocking also revives a remembered cryptographic vault if needed.
pub(crate) async fn set_for_user(shared: &Shared, locked: bool) -> Result<(), CoreError> {
    if !locked {
        let keys = shared
            .config
            .keys
            .as_ref()
            .ok_or_else(crate::keys::no_vault)?;
        match keys.project_secrets().await {
            Ok(_) => {}
            Err(e) if !vw_keys::is_locked(&e) => return Err(crate::keys::key_error(&e)),
            Err(_) => match keys.unlock_remembered().await {
                Ok(true) => {}
                Ok(false) => {
                    let mut error = CoreError::new(
                        ErrorCode::KeyService,
                        "Unlock the vault with its password first.",
                    );
                    error.server_code = Some("passwordRequired".to_owned());
                    return Err(error);
                }
                Err(e) => return Err(crate::keys::key_error(&e)),
            },
        }
    }
    set(shared, locked);
    Ok(())
}
