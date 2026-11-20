//! The one checked path from a forwarded MCP request to its vault key, before and after the person's consent.
//!
//! Every read of a key server's record or key for a forwarded request goes through [`Gate`], in this order:
//! 1. `check`: agent access is not paused, the vault's current record (never a copy) is read, it is not the Debug
//!    bearer, and its secret has no Never permission (default or any project). The result is a checked record:
//!    everything holds except, perhaps, the grant.
//! 2. `permits`: a checked record whose grant holds now (`always`, or this run's plain grant given for this very
//!    record) becomes granted. Without one, the caller may ask the person (the first-use dialog); after the
//!    answer the record is checked again from step 1, must hold a grant, and must still be the record the dialog
//!    showed ([`Shown`]).
//! 3. `value`: the single read of the key's value, which takes a granted record: no read can happen before the
//!    grant was checked on a record read after the consent.
//! 4. After the value read the record is checked once more with steps 1 and 2 (a revoke, a Never, a pause or a
//!    removal while the value was read refuses), and must equal the granted record (a moved one refuses).
//!
//! The steps run in the shared [`crate::consent_gate`] (also used by commands run with a secret); its `Granted` has no
//! other constructor than the gate's own functions, so no caller can reach the value without the checks.
//!
//! Depends on: vw-keys (`KeyService` reads), vw-keyproxy (`McpRefusal`), the parent's grant state.
//! Called by: `mcp_keys_runtime` (the forwarder's resolver); `Shown` also by the parent's `allow_binding`.

use std::future::Future;

use vw_keyproxy::McpRefusal;
use vw_keys::{KeyService, McpBinding};
use vw_vault::SecretString;

use super::McpKeysState;

/// What the consent dialog shows the person about a key server: where the key goes, in which header, and which
/// key. A grant covers only the record that matches what was shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shown {
    upstream: String,
    header: String,
    bearer: bool,
    secret_id: String,
}

impl Shown {
    pub(crate) fn of(binding: &McpBinding) -> Self {
        Self {
            upstream: binding.upstream.clone(),
            header: binding.header.clone(),
            bearer: binding.bearer,
            secret_id: binding.secret_id.clone(),
        }
    }
}

/// The vault reads the gate needs, each mapped to the forwarder's refusal (a trait so the order of checks can be
/// tested with a vault that changes between two reads).
pub(crate) trait Vault: Sync {
    /// The current record of binding `id`.
    fn record(&self, id: &str) -> impl Future<Output = Result<McpBinding, McpRefusal>> + Send;
    /// True when the secret's default or any project permission is Never.
    fn never(&self, secret_id: &str) -> impl Future<Output = Result<bool, McpRefusal>> + Send;
    /// The secret's value. Called only by [`Gate::key`].
    fn value(
        &self,
        secret_id: &str,
    ) -> impl Future<Output = Result<SecretString, McpRefusal>> + Send;
}

impl Vault for KeyService {
    async fn record(&self, id: &str) -> Result<McpBinding, McpRefusal> {
        match self.mcp_binding(id.to_owned()).await {
            Ok(Some(binding)) => Ok(binding),
            Ok(None) => Err(McpRefusal::Unknown),
            Err(e) if vw_keys::is_locked(&e) => Err(McpRefusal::Locked),
            Err(_) => Err(McpRefusal::NotAllowed),
        }
    }

    async fn never(&self, secret_id: &str) -> Result<bool, McpRefusal> {
        // The MCP proxy has no originating session or project: a Never in any project refuses, and Allowed cannot
        // be promoted to a global grant (the server's own grant is still needed).
        let default = self
            .default_entry_permission(secret_id)
            .await
            .map_err(|_| McpRefusal::PermissionsUnreadable)?
            .0;
        if default == vw_keys::EntryPermission::Never {
            return Ok(true);
        }
        Ok(self
            .entry_permissions_for(secret_id)
            .await
            .map_err(|_| McpRefusal::PermissionsUnreadable)?
            .iter()
            .any(|p| p.permission == vw_keys::EntryPermission::Never))
    }

    async fn value(&self, secret_id: &str) -> Result<SecretString, McpRefusal> {
        self.secret_value(secret_id.to_owned(), None)
            .await
            .map_err(|e| {
                if vw_keys::is_locked(&e) {
                    McpRefusal::Locked
                } else {
                    McpRefusal::SecretMissing
                }
            })
    }
}

/// The checks for one forwarded request to binding `id`, as a [`crate::consent_gate::Policy`]: the shared gate runs
/// the steps in the order of the module docs.
pub(crate) struct Gate<'a, V> {
    pub(crate) vault: &'a V,
    pub(crate) grants: &'a McpKeysState,
    /// True while the person has paused agent access.
    pub(crate) paused: &'a (dyn Fn() -> bool + Sync),
    pub(crate) id: &'a str,
}

impl<V: Vault> crate::consent_gate::Policy for Gate<'_, V> {
    type Record = McpBinding;
    /// The first-use dialog stores its grant itself (`allow_binding`); the answer only says it was given.
    type Answer = ();
    type Refusal = McpRefusal;
    type Value = SecretString;

    /// Step 1 (module docs).
    async fn check(&self) -> Result<McpBinding, McpRefusal> {
        if (self.paused)() {
            return Err(McpRefusal::AgentAccessLocked);
        }
        let binding = self.vault.record(self.id).await?;
        // This connection is shared across chats, so even an existing Always grant cannot prove Full access.
        if binding.secret_id == vw_keys::DEBUG_TOKEN_ITEM {
            return Err(McpRefusal::DebugTokenRequiresChat);
        }
        if self.vault.never(&binding.secret_id).await? {
            return Err(McpRefusal::NotAllowed);
        }
        Ok(binding)
    }

    /// Step 2: `always`, or this run's plain grant given for this very record (the answer itself grants nothing).
    fn permits(&self, record: &McpBinding, _answer: Option<&()>) -> bool {
        self.grants.permit(record)
    }

    fn same_as_shown(&self, shown: &McpBinding, now: &McpBinding) -> bool {
        Shown::of(shown) == Shown::of(now)
    }

    fn not_allowed(&self) -> McpRefusal {
        McpRefusal::NotAllowed
    }

    fn changed(&self) -> McpRefusal {
        McpRefusal::NotAllowed
    }

    /// Step 3: the key's value, only for a granted record.
    async fn value(&self, granted: &McpBinding) -> Result<SecretString, McpRefusal> {
        self.vault.value(&granted.secret_id).await
    }
}

impl<V: Vault> Gate<'_, V> {
    /// The whole path (module docs). `ask` shows the person the first-use dialog for a checked record without a
    /// grant and returns whether a grant was given (the record is still read again and checked). Returns the
    /// granted record as it is after the value read, and the value.
    pub(crate) async fn open<A, F>(&self, ask: A) -> Result<(McpBinding, SecretString), McpRefusal>
    where
        A: FnOnce(McpBinding) -> F,
        F: Future<Output = bool>,
    {
        let granted = crate::consent_gate::grant(self, |shown| async move {
            if ask(shown).await {
                Ok(())
            } else {
                Err(McpRefusal::NotAllowed)
            }
        })
        .await?;
        crate::consent_gate::value(self, &granted).await
    }
}

#[cfg(test)]
#[path = "mcp_keys_gate_tests.rs"]
mod tests;
