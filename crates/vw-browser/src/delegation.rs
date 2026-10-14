//! Browser tab-close delegation contract and metadata-only receipts.
//! Depends on serde and the session outcome; called by the core's native authorizer.

use serde::{Deserialize, Serialize};

use crate::sessions::{Actor, Outcome};

/// Acts the agent may perform inside the user's delegated browser task.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BrowserPermissions {
    /// Let the agent close user-origin web tabs without a per-tab confirmation.
    #[serde(default)]
    pub close_user_tabs: bool,
}

/// A recognized act requiring delegation authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DelegatedAct {
    /// Close a user-origin web tab during the delegated task.
    CloseUserTabs,
}

/// What authorized one act; a confirmation never creates a persistent permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DelegationAuthority {
    /// The close-user-tabs permission.
    CloseUserTabs,
    /// A native confirmation answered by the user for this act only.
    ConfirmedByYou,
}

impl BrowserPermissions {
    /// The existing permission for `act`, or `None` when native confirmation is required.
    #[must_use]
    pub fn authority(&self, act: DelegatedAct) -> Option<DelegationAuthority> {
        match act {
            DelegatedAct::CloseUserTabs if self.close_user_tabs => {
                Some(DelegationAuthority::CloseUserTabs)
            }
            DelegatedAct::CloseUserTabs => None,
        }
    }
}

/// One retained session receipt. Wording, form values, paths and query strings are never stored here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DelegationReceipt {
    /// Owning session.
    pub session_id: String,
    /// Completion time in milliseconds since the Unix epoch.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub at_ms: u64,
    /// Page host only.
    pub site: String,
    /// Recognized act.
    pub act: DelegatedAct,
    /// Authority used for the attempt.
    pub authority: DelegationAuthority,
    /// Whether the authorized act ran, failed, was refused by a changed page, or was stopped.
    pub outcome: Outcome,
}

/// Core-owned page admission and tab-close receipt storage.
pub trait DelegationAuthorizer: Send + Sync {
    /// Rechecks the concrete page's actor rights at the final native input boundary.
    fn page_authorized(&self, _page_key: &str, _actor: Actor) -> bool {
        true
    }

    /// Persists a completed attempt's metadata. Test-only authorizers may keep receipts in session memory.
    ///
    /// # Errors
    /// The storage failure; callers retain the receipt in memory and report the failure after the act.
    fn record(&self, _receipt: &DelegationReceipt) -> Result<(), String> {
        Ok(())
    }

    /// Removes a deleted session's durable receipts.
    ///
    /// # Errors
    /// The storage failure, which the core reports without discarding other sessions' receipts.
    fn forget(&self, _session_id: &str) -> Result<(), String> {
        Ok(())
    }
}
